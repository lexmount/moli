use super::*;

#[test]
fn character_data_constructors_parse_webidl_data() {
    let mut vm = new_storage_test_vm("https://character-data-constructors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const text = new Text('hello');
              const emptyText = new Text();
              const comment = new Comment(null);
              return [
                text.data,
                emptyText.data,
                comment.data
              ].join('|');
            })()
            "#,
        )
        .expect("Text and Comment constructors should parse WebIDL data");

    assert_eq!(result, "hello||null");
}
#[test]
fn character_data_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://character-data-methods.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const text = document.createTextNode('abcdef');
              function probe(callback) {
                try {
                  return callback();
                } catch (error) {
                  return 'throw:' + error.name;
                }
              }
              text.appendData(null);
              text.insertData('1', 'X');
              text.deleteData(2, '2');
              text.replaceData(0, 1, undefined);
              const slice = text.substringData(1, 4);
              return [
                text.data,
                slice,
                probe(() => text.substringData(99, 1)),
                probe(() => text.appendData(Symbol()))
              ].join('|');
            })()
            "#,
        )
        .expect("CharacterData methods should parse WebIDL arguments");

    assert_eq!(
        result,
        "undefinedXdefnull|ndef|throw:IndexSizeError|throw:TypeError"
    );
}
#[test]
fn character_data_edit_methods_queue_noop_mutation_records() {
    let mut vm = new_storage_test_vm("https://character-data-noop-mutations.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const text = document.createTextNode('CHANGED');
              const observer = new MutationObserver(() => {});
              observer.observe(text, { characterData: true, characterDataOldValue: true });
              text.appendData('');
              text.insertData(2, '');
              text.deleteData(0, 0);
              text.replaceData(0, 2, 'CH');
              return observer.takeRecords().map(record => record.oldValue).join('|');
            })()
            "#,
        )
        .expect("CharacterData no-op edit methods should queue mutation records");

    assert_eq!(result, "CHANGED|CHANGED|CHANGED|CHANGED");
}
#[test]
fn character_data_methods_preserve_split_surrogate_pairs() {
    let mut vm = new_storage_test_vm("https://character-data-surrogates.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const codes = value => Array.from(
                { length: value.length },
                (_, index) => value.charCodeAt(index).toString(16)
              );
              const text = document.createTextNode('test');
              text.data = '🌠 test 🌠 TEST';
              const substring = codes(text.substringData(1, 8));
              text.replaceData(1, 4, '--');
              const replaced = codes(text.data);

              const comment = document.createComment('test');
              comment.data = '🌠 test 🌠 TEST';
              comment.insertData(1, '\uDF1F ');
              const inserted = codes(comment.data);
              comment.data = '🌠 test 🌠 TEST';
              comment.deleteData(1, 4);
              const deleted = codes(comment.data);

              return JSON.stringify({ substring, replaced, inserted, deleted });
            })()
            "#,
        )
        .expect("CharacterData methods should preserve split surrogate code units");

    assert_eq!(
        result,
        r#"{"substring":["df20","20","74","65","73","74","20","d83c"],"replaced":["d83c","2d","2d","73","74","20","d83c","df20","20","54","45","53","54"],"inserted":["d83c","df1f","20","df20","20","74","65","73","74","20","d83c","df20","20","54","45","53","54"],"deleted":["d83c","73","74","20","d83c","df20","20","54","45","53","54"]}"#
    );
}
#[test]
fn live_character_data_accessors_parse_webidl_values() {
    let mut vm = new_storage_test_vm("https://character-data-accessor-webidl.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const text = document.createTextNode('seed');
  const comment = document.createComment('note');
  text.data = {
    toString() {
      return 'data-object';
    }
  };
  const dataObject = `${text.data}:${text.nodeValue}`;
  text.data = null;
  const dataNull = `${text.data}:${text.nodeValue}`;
  const dataSymbol = probe(() => { text.data = Symbol('data'); });
  comment.nodeValue = {
    toString() {
      return 'node-object';
    }
  };
  const nodeObject = `${comment.data}:${comment.nodeValue}`;
  comment.nodeValue = null;
  const nodeNull = `${comment.data}:${comment.nodeValue}`;
  comment.nodeValue = undefined;
  const nodeUndefined = `${comment.data}:${comment.nodeValue}`;
  const nodeSymbol = probe(() => { comment.nodeValue = Symbol('nodeValue'); });
  return [
    dataObject,
    dataNull,
    dataSymbol,
    nodeObject,
    nodeNull,
    nodeUndefined,
    nodeSymbol
  ].join('|');
})()
"##,
        )
        .expect("live character-data accessors should parse WebIDL values");

    assert_eq!(
        result,
        "data-object:data-object|:|throw:TypeError|node-object:node-object|:|:|throw:TypeError"
    );
}
#[test]
fn text_control_selection_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://forms-selection-webidl.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const input = document.createElement('input');
              input.type = 'text';
              input.value = 'abcdef';
              function probe(callback) {
                try {
                  return String(callback());
                } catch (error) {
                  return 'throw:' + error.name;
                }
              }
              input.setSelectionRange(undefined, '3.9', null);
              const first = `${input.selectionStart},${input.selectionEnd},${input.selectionDirection}`;
              const missing = probe(() => input.setSelectionRange(1));
              const symbolDirection = probe(() => input.setSelectionRange(0, 1, Symbol()));
              const afterFailed = `${input.selectionStart},${input.selectionEnd},${input.selectionDirection}`;
              input.setSelectionRange(0, 1, 'backward');
              const backward = `${input.selectionStart},${input.selectionEnd},${input.selectionDirection}`;
              input.selectionDirection = 'forward';
              const forward = `${input.selectionStart},${input.selectionEnd},${input.selectionDirection}`;
              input.setSelectionRange(-1, 2);
              return [
                first,
                missing,
                symbolDirection,
                afterFailed,
                backward,
                forward,
                `${input.selectionStart},${input.selectionEnd},${input.selectionDirection}`
              ].join('|');
            })()
            "#,
        )
        .expect("text control selection methods should parse WebIDL arguments");

    assert_eq!(
        result,
        "0,3,none|throw:TypeError|throw:TypeError|0,3,none|0,1,backward|0,1,forward|2,2,none"
    );
}

#[test]
fn text_control_selection_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://forms-selection-prototypes.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} descriptor missing`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(typeof descriptor.set === "function", `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              for (const prototype of [HTMLInputElement.prototype, HTMLTextAreaElement.prototype]) {
                accessor(prototype, "selectionStart");
                accessor(prototype, "selectionEnd");
                accessor(prototype, "selectionDirection");
              }

              const input = document.createElement("input");
              input.type = "text";
              input.value = "abcdef";
              const textarea = document.createElement("textarea");
              textarea.value = "ghijkl";
              const detachedDoc = document.implementation.createHTMLDocument("");
              const detachedInput = detachedDoc.createElement("input");
              detachedInput.type = "text";
              detachedInput.value = "mnop";
              const detachedTextarea = detachedDoc.createElement("textarea");
              detachedTextarea.value = "qrst";

              for (const element of [input, textarea, detachedInput, detachedTextarea]) {
                for (const name of ["selectionStart", "selectionEnd", "selectionDirection"]) {
                  assert(!own(element, name), `${element.localName}.${name} should not be own before set`);
                }
              }

              input.selectionStart = 1;
              input.selectionEnd = 3;
              input.selectionDirection = "backward";
              textarea.selectionStart = 2;
              textarea.selectionEnd = 4;
              detachedInput.selectionStart = 1;
              detachedInput.selectionEnd = 2;
              detachedTextarea.selectionStart = 1;
              detachedTextarea.selectionEnd = 3;

              for (const element of [input, textarea, detachedInput, detachedTextarea]) {
                for (const name of ["selectionStart", "selectionEnd", "selectionDirection"]) {
                  assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
                  assert(delete element[name], `${element.localName}.${name} delete`);
                  assert(!own(element, name), `${element.localName}.${name} should not be own after delete`);
                }
              }

              return [
                input.selectionStart,
                input.selectionEnd,
                input.selectionDirection,
                textarea.selectionStart,
                textarea.selectionEnd,
                detachedInput.selectionStart,
                detachedInput.selectionEnd,
                detachedTextarea.selectionStart,
                detachedTextarea.selectionEnd
              ].join("|");
            })()
            "#,
        )
        .expect("text control selection prototype accessors should evaluate");

    assert_eq!(result, "1|3|backward|2|4|1|2|1|3");
}

#[test]
fn text_control_value_setters_reset_selection_only_when_api_value_changes() {
    let mut vm = new_storage_test_vm("https://forms-selection-value-reset.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const input = document.createElement('input');
              input.type = 'text';
              input.value = 'hello';
              input.setSelectionRange(1, 3, 'backward');
              input.value = 'hello';
              const inputSame = [
                input.selectionStart,
                input.selectionEnd,
                input.selectionDirection
              ].join(':');
              input.value = 'hello2';
              const inputDifferent = [
                input.selectionStart,
                input.selectionEnd,
                input.selectionDirection
              ].join(':');

              const textarea = document.createElement('textarea');
              textarea.value = 'hell\no';
              textarea.setSelectionRange(1, 3, 'backward');
              textarea.value = 'hell\r\no';
              const textareaSame = [
                textarea.selectionStart,
                textarea.selectionEnd,
                textarea.selectionDirection
              ].join(':');
              textarea.value = 'hello2';
              const textareaDifferent = [
                textarea.selectionStart,
                textarea.selectionEnd,
                textarea.selectionDirection
              ].join(':');

              return [
                inputSame,
                inputDifferent,
                textareaSame,
                textareaDifferent
              ].join('|');
            })()
            "#,
        )
        .expect("text control value selection reset should evaluate");

    assert_eq!(result, "1:3:backward|6:6:none|1:3:backward|6:6:none");
}
#[test]
fn text_control_default_value_type_and_reset_paths_clamp_selection() {
    let mut vm = new_storage_test_vm("https://forms-selection-clamp-reset.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const textarea = document.createElement('textarea');
              textarea.textContent = 'foobar';
              textarea.selectionStart = 3;
              textarea.selectionEnd = 5;
              textarea.firstChild.remove();
              const textareaAfterRemove = `${textarea.selectionStart}:${textarea.selectionEnd}`;
              textarea.appendChild(document.createTextNode('foobar'));
              textarea.selectionStart = 3;
              textarea.selectionEnd = 5;
              textarea.defaultValue = 'foobar';
              const textareaAfterDefault = `${textarea.selectionStart}:${textarea.selectionEnd}`;

              const urlInput = document.createElement('input');
              urlInput.type = 'text';
              urlInput.value = 'http://example.com   ';
              urlInput.type = 'url';
              const urlAfterType = `${urlInput.selectionStart}:${urlInput.selectionEnd}`;

              const colorInput = document.createElement('input');
              colorInput.type = 'text';
              colorInput.value = '#123456xx';
              colorInput.type = 'color';
              colorInput.type = 'text';
              const colorAfterType = `${colorInput.selectionStart}:${colorInput.selectionEnd}`;

              const checkboxFromNumber = document.createElement('input');
              checkboxFromNumber.type = 'number';
              checkboxFromNumber.value = 'bad';
              checkboxFromNumber.type = 'checkbox';
              const checkboxAfterType = checkboxFromNumber.value;

              const radioFromDate = document.createElement('input');
              radioFromDate.type = 'date';
              radioFromDate.value = 'not-a-date';
              radioFromDate.type = 'radio';
              const radioAfterType = radioFromDate.value;

              const hiddenToText = document.createElement('input');
              hiddenToText.type = 'hidden';
              hiddenToText.value = 'abcdefghij';
              hiddenToText.type = 'text';
              const hiddenToTextSelection = `${hiddenToText.selectionStart}:${hiddenToText.selectionEnd}`;

              const checkboxToSearch = document.createElement('input');
              checkboxToSearch.type = 'checkbox';
              checkboxToSearch.type = 'search';
              const checkboxToSearchSelection = `${checkboxToSearch.selectionStart}:${checkboxToSearch.selectionEnd}`;

              const form = document.createElement('form');
              const resetInput = document.createElement('input');
              resetInput.type = 'text';
              resetInput.setAttribute('value', 'abc');
              form.append(resetInput);
              resetInput.value = 'abcde';
              form.reset();
              const afterFormReset = `${resetInput.value}:${resetInput.selectionStart}:${resetInput.selectionEnd}`;

              return [
                textareaAfterRemove,
                textareaAfterDefault,
                urlAfterType,
                colorAfterType,
                checkboxAfterType,
                radioAfterType,
                hiddenToTextSelection,
                checkboxToSearchSelection,
                afterFormReset
              ].join('|');
            })()
            "#,
        )
        .expect("text control selection clamp/reset paths should evaluate");

    assert_eq!(result, "0:0|0:0|18:18|0:0|on|on|0:0|0:0|abc:3:3");
}

#[test]
fn input_click_activation_state_and_type_change_defaults_match_browser_order() {
    let mut vm = new_storage_test_vm("https://forms-input-click-activation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const results = [];

  const checkbox = document.createElement('input');
  checkbox.type = 'checkbox';
  host.appendChild(checkbox);
  const checkboxEvents = [];
  checkbox.addEventListener('click', event => {
    checkboxEvents.push(`click:${checkbox.checked}`);
    event.preventDefault();
    checkboxEvents.push(`prevented:${checkbox.checked}`);
  });
  checkbox.click();
  checkboxEvents.push(`after:${checkbox.checked}`);
  results.push(checkboxEvents.join(','));

  const dispatched = document.createElement('input');
  dispatched.type = 'checkbox';
  host.appendChild(dispatched);
  const dispatchedEvents = [];
  dispatched.addEventListener('click', () => dispatchedEvents.push(`click:${dispatched.checked}`));
  dispatched.addEventListener('input', () => dispatchedEvents.push('input'));
  dispatched.addEventListener('change', () => dispatchedEvents.push('change'));
  const allowed = dispatched.dispatchEvent(new MouseEvent('click', {
    bubbles: true,
    cancelable: true
  }));
  results.push(`${allowed}:${dispatched.checked}:${dispatchedEvents.join(',')}`);

  const form = document.createElement('form');
  const typeChanged = document.createElement('input');
  typeChanged.type = 'text';
  typeChanged.name = 'n1';
  typeChanged.value = 'v1';
  form.appendChild(typeChanged);
  host.appendChild(form);
  const submits = [];
  form.addEventListener('submit', event => {
    event.preventDefault();
    submits.push(`${typeChanged.type}:${typeChanged.value}`);
  });
  typeChanged.addEventListener('click', () => {
    typeChanged.type = 'submit';
    typeChanged.value = 'v2';
  });
  typeChanged.click();
  results.push(submits.join(','));

  const radioForm = document.createElement('form');
  radioForm.innerHTML = '<input id="radio1" type="radio" name="group" checked><input id="radio2" type="radio" name="group">';
  host.appendChild(radioForm);
  const radio1 = radioForm.querySelector('#radio1');
  const radio2 = radioForm.querySelector('#radio2');
  const radioEvents = [];
  radio2.addEventListener('click', event => {
    radioEvents.push(`${radio1.checked}:${radio2.checked}`);
    event.preventDefault();
  });
  radio2.click();
  radioEvents.push(`${radio1.checked}:${radio2.checked}`);
  results.push(radioEvents.join(','));

  const singleRadio = document.createElement('input');
  singleRadio.type = 'radio';
  host.appendChild(singleRadio);
  singleRadio.addEventListener('click', event => event.preventDefault());
  singleRadio.click();
  results.push(String(singleRadio.checked));

  return results.join('|');
})()
"#,
        )
        .expect("input click activation probe should evaluate");

    assert_eq!(
        result,
        "click:true,prevented:true,after:false|true:true:click:true,input,change|submit:v2|false:true,true:false|false"
    );
}

#[test]
fn checkbox_click_activation_tracks_trust_and_indeterminate_state() {
    let mut vm = new_storage_test_vm("https://forms-checkbox-click-activation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const results = [];

  for (const initiallyChecked of [false, true]) {
    const checkbox = document.createElement('input');
    checkbox.type = 'checkbox';
    checkbox.checked = initiallyChecked;
    checkbox.indeterminate = true;
    host.appendChild(checkbox);
    const events = [];
    checkbox.addEventListener('click', event => {
      events.push(`click:${event.isTrusted}:${checkbox.checked}:${checkbox.indeterminate}`);
    });
    checkbox.addEventListener('input', event => {
      events.push(`input:${event.isTrusted}:${checkbox.checked}:${checkbox.indeterminate}`);
    });
    checkbox.addEventListener('change', event => {
      events.push(`change:${event.isTrusted}:${checkbox.checked}:${checkbox.indeterminate}`);
    });
    checkbox.click();
    events.push(`after:${checkbox.checked}:${checkbox.indeterminate}`);
    results.push(events.join(','));
  }

  const canceled = document.createElement('input');
  canceled.type = 'checkbox';
  host.appendChild(canceled);
  const canceledEvents = [];
  canceled.addEventListener('click', event => {
    canceledEvents.push(`click:${canceled.checked}:${canceled.indeterminate}`);
    canceled.indeterminate = true;
    event.preventDefault();
    canceledEvents.push(`prevented:${canceled.checked}:${canceled.indeterminate}`);
  });
  canceled.click();
  canceledEvents.push(`after:${canceled.checked}:${canceled.indeterminate}`);
  results.push(canceledEvents.join(','));

  return results.join('|');
})()
"#,
        )
        .expect("checkbox activation trust and indeterminate probe should evaluate");

    assert_eq!(
        result,
        "click:false:true:false,input:true:true:false,change:true:true:false,after:true:false|click:false:false:false,input:true:false:false,change:true:false:false,after:false:false|click:true:false,prevented:true:true,after:false:false"
    );
}

#[test]
fn detached_checkbox_and_radio_activation_omits_input_and_change_events() {
    let mut vm = new_storage_test_vm("https://forms-detached-activation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const results = [];
  for (const type of ['checkbox', 'radio']) {
    for (const dispatch of ['click', 'dispatchEvent']) {
      const input = document.createElement('input');
      input.type = type;
      const events = [];
      input.addEventListener('input', () => events.push('input'));
      input.addEventListener('change', () => events.push('change'));
      if (dispatch === 'click') {
        input.click();
      } else {
        input.dispatchEvent(new MouseEvent('click'));
      }
      results.push(`${type}:${dispatch}:${input.checked}:${events.join(',')}`);
    }
  }
  return results.join('|');
})()
"#,
        )
        .expect("detached input activation probe should evaluate");

    assert_eq!(
        result,
        "checkbox:click:true:|checkbox:dispatchEvent:true:|radio:click:true:|radio:dispatchEvent:true:"
    );
}

#[test]
fn dispatched_mouse_click_selects_one_activation_target() {
    let mut vm = new_storage_test_vm("https://forms-dispatched-activation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const results = [];

  const wrongClass = document.createElement('input');
  wrongClass.type = 'checkbox';
  const spoofedMouseEvent = new Event('click');
  Object.setPrototypeOf(spoofedMouseEvent, MouseEvent.prototype);
  wrongClass.dispatchEvent(spoofedMouseEvent);
  results.push(`wrong:${wrongClass.checked}`);

  const branded = document.createElement('input');
  branded.type = 'checkbox';
  const mouseEventWithoutMousePrototype = new MouseEvent('click');
  Object.setPrototypeOf(mouseEventWithoutMousePrototype, Event.prototype);
  branded.dispatchEvent(mouseEventWithoutMousePrototype);
  results.push(`branded:${branded.checked}`);

  const parent = document.createElement('input');
  parent.type = 'checkbox';
  parent.append(new Text('target'));
  parent.firstChild.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  results.push(`parent:${parent.checked}`);

  const nonBubblingParent = document.createElement('input');
  nonBubblingParent.type = 'checkbox';
  nonBubblingParent.append(new Text('target'));
  nonBubblingParent.firstChild.dispatchEvent(new MouseEvent('click'));
  results.push(`nonbubbling:${nonBubblingParent.checked}`);

  for (const type of ['checkbox', 'radio']) {
    const input = document.createElement('input');
    input.type = type;
    input.disabled = true;
    input.dispatchEvent(new MouseEvent('click'));
    results.push(`disabled-${type}:${input.checked}`);

    const canceled = document.createElement('input');
    canceled.type = type;
    canceled.disabled = true;
    let checkedDuringClick = false;
    canceled.onclick = event => {
      checkedDuringClick = canceled.checked;
      event.preventDefault();
    };
    canceled.dispatchEvent(new MouseEvent('click', { cancelable: true }));
    results.push(`canceled-${type}:${checkedDuringClick}:${canceled.checked}`);
  }

  return results.join('|');
})()
"#,
        )
        .expect("dispatched mouse click activation probe should evaluate");

    assert_eq!(
        result,
        "wrong:false|branded:true|parent:true|nonbubbling:false|disabled-checkbox:true|canceled-checkbox:true:false|disabled-radio:true|canceled-radio:true:false"
    );
}

#[tokio::test]
async fn text_control_selection_mutations_queue_select_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://forms-selection-select-event.test/",
        &loader,
    );
    macro_rules! advance_until_select_event_count {
        ($expected:expr, $context:literal) => {{
            for _ in 0..8 {
                if vm
                    .eval("globalThis.__textControlSelectEvents.length")
                    .expect("text control select event count should evaluate")
                    == $expected.to_string()
                {
                    break;
                }
                if !vm
                    .run_one_oldest_ready_page_task_executor_turn(&loader)
                    .await
                    .expect($context)
                {
                    break;
                }
            }
        }};
    }

    let result = vm
        .eval(
            r#"
            (() => {
              const input = document.createElement('input');
              input.type = 'text';
              input.value = 'foobar';
              globalThis.__textControlSelectInput = input;
              globalThis.__textControlSelectEvents = [];
              input.addEventListener('select', (event) => {
                globalThis.__textControlSelectEvents.push([
                  event.type,
                  event.isTrusted,
                  event.bubbles,
                  event.cancelable,
                  input.selectionStart,
                  input.selectionEnd
                ].join(':'));
              });

              input.setSelectionRange(0, input.value.length);
              return [
                globalThis.__textControlSelectEvents.length,
                input.selectionStart,
                input.selectionEnd
              ].join(':');
            })()
            "#,
        )
        .expect("selection mutation setup should evaluate");

    assert_eq!(result, "0:0:6");
    advance_until_select_event_count!(1, "wait driver should advance queued select event");
    assert_eq!(
        vm.eval("globalThis.__textControlSelectEvents.join('|')")
            .expect("queued select event log should evaluate"),
        "select:true:true:false:0:6"
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const input = globalThis.__textControlSelectInput;
              globalThis.__textControlSelectEvents = [];
              input.setRangeText('xy', 0, 3, 'select');
              return [
                globalThis.__textControlSelectEvents.length,
                input.value,
                input.selectionStart,
                input.selectionEnd
              ].join(':');
            })()
            "#,
        )
        .expect("setRangeText select-event setup should evaluate");

    assert_eq!(result, "0:xybar:0:2");
    advance_until_select_event_count!(
        1,
        "wait driver should advance queued setRangeText select event"
    );
    assert_eq!(
        vm.eval("globalThis.__textControlSelectEvents.join('|')")
            .expect("queued setRangeText select event log should evaluate"),
        "select:true:true:false:0:2"
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const input = globalThis.__textControlSelectInput;
              globalThis.__textControlSelectEvents = [];
              input.setSelectionRange(1, 3);
              input.setSelectionRange(2, 4);
              return [
                globalThis.__textControlSelectEvents.length,
                input.selectionStart,
                input.selectionEnd
              ].join(':');
            })()
            "#,
        )
        .expect("coalesced select-event setup should evaluate");

    assert_eq!(result, "0:2:4");
    advance_until_select_event_count!(1, "wait driver should advance coalesced select event");
    assert_eq!(
        vm.eval("globalThis.__textControlSelectEvents.join('|')")
            .expect("coalesced select event log should evaluate"),
        "select:true:true:false:2:4"
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const input = globalThis.__textControlSelectInput;
              globalThis.__textControlSelectEvents = [];
              input.selectionStart = 1;
              input.selectionDirection = 'backward';
              return [
                globalThis.__textControlSelectEvents.length,
                input.selectionStart,
                input.selectionEnd,
                input.selectionDirection
              ].join(':');
            })()
            "#,
        )
        .expect("selection accessor select-event setup should evaluate");

    assert_eq!(result, "0:1:4:backward");
    advance_until_select_event_count!(1, "wait driver should advance accessor select event");
    assert_eq!(
        vm.eval("globalThis.__textControlSelectEvents.join('|')")
            .expect("accessor select event log should evaluate"),
        "select:true:true:false:1:4"
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const input = globalThis.__textControlSelectInput;
              globalThis.__textControlSelectEvents = [];
              input.select();
              return [
                globalThis.__textControlSelectEvents.length,
                input.selectionStart,
                input.selectionEnd
              ].join(':');
            })()
            "#,
        )
        .expect("select() async select-event setup should evaluate");

    assert_eq!(result, "0:0:5");
    advance_until_select_event_count!(1, "wait driver should advance select() select event");
    assert_eq!(
        vm.eval("globalThis.__textControlSelectEvents.join('|')")
            .expect("select() select event log should evaluate"),
        "select:true:true:false:0:5"
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const input = globalThis.__textControlSelectInput;
              globalThis.__textControlSelectEvents = [];
              input.select();
              return [
                globalThis.__textControlSelectEvents.length,
                input.selectionStart,
                input.selectionEnd
              ].join(':');
            })()
            "#,
        )
        .expect("repeated select() async select-event setup should evaluate");

    assert_eq!(result, "0:0:5");
    let _ = vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await
        .expect("wait driver should check repeated select() select event");
    assert_eq!(
        vm.eval("globalThis.__textControlSelectEvents.join('|')")
            .expect("repeated select() select event log should evaluate"),
        ""
    );
}
#[tokio::test]
async fn text_control_clone_resets_selection_but_still_queues_select_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://forms-selection-clone.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const input = document.createElement('input');
              input.type = 'text';
              input.value = 'foobar';
              input.setSelectionRange(2, 4, 'backward');

              const textarea = document.createElement('textarea');
              textarea.value = 'foobar';
              textarea.setSelectionRange(1, 5, 'forward');

              const inputClone = input.cloneNode(true);
              const textareaClone = textarea.cloneNode(true);
              globalThis.__textControlInputClone = inputClone;
              globalThis.__textControlTextareaClone = textareaClone;
              globalThis.__textControlCloneSelectEvents = [];
              globalThis.__textControlCloneSelectionEvents = [];
              inputClone.addEventListener('selectionchange', () => {
                globalThis.__textControlCloneSelectionEvents.push('input');
              });
              textareaClone.addEventListener('selectionchange', () => {
                globalThis.__textControlCloneSelectionEvents.push('textarea');
              });
              inputClone.addEventListener('select', (event) => {
                globalThis.__textControlCloneSelectEvents.push([
                  'input',
                  event.isTrusted,
                  event.bubbles,
                  event.cancelable,
                  inputClone.selectionStart,
                  inputClone.selectionEnd
                ].join(':'));
              });
              textareaClone.addEventListener('select', (event) => {
                globalThis.__textControlCloneSelectEvents.push([
                  'textarea',
                  event.isTrusted,
                  event.bubbles,
                  event.cancelable,
                  textareaClone.selectionStart,
                  textareaClone.selectionEnd
                ].join(':'));
              });

              const beforeInput = `${inputClone.selectionStart}:${inputClone.selectionEnd}`;
              const beforeTextarea = `${textareaClone.selectionStart}:${textareaClone.selectionEnd}`;
              inputClone.setSelectionRange(0, inputClone.value.length);
              textareaClone.selectionEnd = textareaClone.value.length;
              const afterInput = `${inputClone.selectionStart}:${inputClone.selectionEnd}`;
              const afterTextarea = `${textareaClone.selectionStart}:${textareaClone.selectionEnd}`;

              return [
                `${inputClone.value}:${beforeInput}:${afterInput}:${inputClone.selectionDirection}`,
                `${textareaClone.value}:${beforeTextarea}:${afterTextarea}:${textareaClone.selectionDirection}`,
                globalThis.__textControlCloneSelectEvents.length,
                globalThis.__textControlCloneSelectionEvents.join(',')
              ].join('|');
            })()
            "#,
        )
        .expect("text control clone setup should evaluate");

    assert_eq!(result, "foobar:0:0:0:6:none|foobar:0:0:0:6:none|0|");
    for _ in 0..8 {
        if vm
            .eval("globalThis.__textControlCloneSelectEvents.length")
            .expect("clone select event count should evaluate")
            == "2"
            && vm
                .eval("globalThis.__textControlCloneSelectionEvents.length")
                .expect("clone selectionchange event count should evaluate")
                == "2"
        {
            break;
        }
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance queued clone select events")
        {
            break;
        }
    }
    assert_eq!(
        vm.eval("globalThis.__textControlCloneSelectEvents.join('|')")
            .expect("queued clone select event log should evaluate"),
        "input:true:true:false:0:6|textarea:true:true:false:0:6"
    );
    assert_eq!(
        vm.eval("globalThis.__textControlCloneSelectionEvents.join('|')")
            .expect("queued clone selectionchange event log should evaluate"),
        "input|textarea"
    );
}
#[tokio::test]
async fn text_control_selectionchange_bubbles_across_shadow_and_exec_delete_targets_document() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://forms-selectionchange-shadow-text-control.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('div');
  (document.body || document.documentElement || document).append(host);
  const shadow = host.attachShadow({ mode: 'open' });
  const input = document.createElement('input');
  input.value = 'hello';
  shadow.append(input);
  globalThis.__shadowTextControl = input;
  globalThis.__shadowTextControlInputEvents = [];
  globalThis.__shadowTextControlDocumentEvents = [];
  input.addEventListener('selectionchange', event => {
    globalThis.__shadowTextControlInputEvents.push(event.target === input);
  });
  document.addEventListener('selectionchange', event => {
    globalThis.__shadowTextControlDocumentEvents.push(event.target === document ? 'document' : 'bubble');
  });
  input.focus();
  input.setSelectionRange(0, 1);
  return [
    input.selectionStart,
    input.selectionEnd,
    globalThis.__shadowTextControlInputEvents.length,
    globalThis.__shadowTextControlDocumentEvents.length
  ].join('|');
})()
"#,
        )
        .expect("shadow text control selectionchange setup should evaluate");

    assert_eq!(result, "0|1|0|0");
    for _ in 0..8 {
        if vm
            .eval("globalThis.__shadowTextControlDocumentEvents.length")
            .expect("document selectionchange event count should evaluate")
            == "1"
        {
            break;
        }
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance queued shadow selectionchange")
        {
            break;
        }
    }
    assert_eq!(
        vm.eval(
            "globalThis.__shadowTextControlInputEvents.join(',') + '|' + globalThis.__shadowTextControlDocumentEvents.join(',')",
        )
        .expect("shadow selectionchange event log should evaluate"),
        "true|bubble"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const input = globalThis.__shadowTextControl;
  globalThis.__shadowTextControlInputEvents = [];
  globalThis.__shadowTextControlDocumentEvents = [];
  input.setSelectionRange(5, 5);
  return [input.selectionStart, input.selectionEnd].join('|');
})()
"#,
        )
        .expect("shadow text control caret setup should evaluate");
    assert_eq!(result, "5|5");
    for _ in 0..8 {
        if vm
            .eval("globalThis.__shadowTextControlDocumentEvents.length")
            .expect("document selectionchange pre-delete count should evaluate")
            == "1"
        {
            break;
        }
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance caret selectionchange")
        {
            break;
        }
    }

    let result = vm
        .eval(
            r#"
(() => {
  const input = globalThis.__shadowTextControl;
  globalThis.__shadowTextControlInputEvents = [];
  globalThis.__shadowTextControlDocumentEvents = [];
  const returned = document.execCommand('delete');
  return [
    returned,
    input.value,
    input.selectionStart,
    input.selectionEnd,
    globalThis.__shadowTextControlInputEvents.length,
    globalThis.__shadowTextControlDocumentEvents.length
  ].join('|');
})()
"#,
        )
        .expect("shadow text control execCommand delete should evaluate");

    assert_eq!(result, "true|hell|4|4|0|0");
    for _ in 0..8 {
        if vm
            .eval("globalThis.__shadowTextControlDocumentEvents.length")
            .expect("document selectionchange delete count should evaluate")
            == "1"
        {
            break;
        }
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance document selectionchange")
        {
            break;
        }
    }
    assert_eq!(
        vm.eval(
            "globalThis.__shadowTextControlInputEvents.length + '|' + globalThis.__shadowTextControlDocumentEvents.join(',')",
        )
        .expect("document selectionchange delete log should evaluate"),
        "0|document"
    );
}

#[test]
fn exec_command_insert_html_enforces_maxlength_in_utf16_units() {
    let mut vm = new_storage_test_vm("https://exec-command-insert-html-maxlength.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const input = document.createElement('input');
  input.setAttribute('maxlength', ' +10tail');
  body.append(input);
  const events = [];
  input.addEventListener('beforeinput', () => events.push('beforeinput'));
  input.addEventListener('input', () => events.push('input'));

  const supported = document.queryCommandSupported('InsertHTML');
  const enabledBeforeFocus = document.queryCommandEnabled('InsertHTML');
  input.focus();
  const enabled = document.queryCommandEnabled('InsertHTML');
  const returned = document.execCommand('InsertHTML', false, '👨‍👩‍👧‍👦');

  const markupInput = document.createElement('input');
  body.append(markupInput);
  markupInput.focus();
  const markupReturned = document.execCommand(
    'InsertHTML',
    false,
    '<b>A</b><br>B<script>C</script>&amp;'
  );

  const rangeInput = document.createElement('input');
  rangeInput.value = 'A😀B';
  body.append(rangeInput);
  rangeInput.setRangeText('x', 1, 3, 'end');

  input.readOnly = true;
  input.focus();
  const enabledReadonly = document.queryCommandEnabled('InsertHTML');
  const readonlyReturned = document.execCommand('InsertHTML', false, 'x');

  return JSON.stringify({
    supported,
    enabledBeforeFocus,
    enabled,
    returned,
    value: input.value,
    valueLength: input.value.length,
    selectionStart: input.selectionStart,
    selectionEnd: input.selectionEnd,
    events,
    markupReturned,
    markupValue: markupInput.value,
    rangeValue: rangeInput.value,
    rangeSelection: [rangeInput.selectionStart, rangeInput.selectionEnd],
    enabledReadonly,
    readonlyReturned
  });
})()
"#,
        )
        .expect("execCommand InsertHTML maxlength probe should evaluate");

    assert_eq!(
        result,
        r#"{"supported":true,"enabledBeforeFocus":false,"enabled":true,"returned":true,"value":"👨‍👩‍👧‍","valueLength":9,"selectionStart":9,"selectionEnd":9,"events":["beforeinput","input"],"markupReturned":true,"markupValue":"A BC&","rangeValue":"AxB","rangeSelection":[2,2],"enabledReadonly":false,"readonlyReturned":false}"#
    );
}

#[test]
fn input_file_value_setter_rejects_non_empty_values() {
    let mut vm = new_storage_test_vm("https://forms-input-file-value.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const input = document.createElement('input');
              input.type = 'file';
              function probe(value) {
                try {
                  input.value = value;
                  return `ok:${input.value}:${input.getAttribute('value')}`;
                } catch (error) {
                  return `throw:${error.name}:${input.value}:${input.getAttribute('value')}`;
                }
              }
              const empty = probe('');
              const nullValue = probe(null);
              const stringValue = probe('foo');
              const numberValue = probe(10);
              const undefinedValue = probe(undefined);
              input.setAttribute('value', 'bar');
              const withAttribute = probe('');
              return [
                empty,
                nullValue,
                stringValue,
                numberValue,
                undefinedValue,
                withAttribute
              ].join('|');
            })()
            "#,
        )
        .expect("file input value setter should evaluate");

    assert_eq!(
        result,
        "ok::null|ok::null|throw:InvalidStateError::null|throw:InvalidStateError::null|throw:InvalidStateError::null|ok::bar"
    );
}

#[test]
fn input_type_change_preserves_clean_default_value_source() {
    let mut vm = new_storage_test_vm("https://forms-input-type-clean-value.test/");

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
              function run(focus) {
                const input = document.createElement('input');
                input.type = 'color';
                document.body.appendChild(input);
                if (focus) {
                  input.focus();
                }
                const colorDefault = input.value;
                input.type = 'text';
                const cleanTextValue = input.value;
                input.type = 'color';
                input.value = '#ffffff';
                input.type = 'text';
                return [
                  colorDefault,
                  cleanTextValue,
                  input.value,
                  !focus || document.activeElement === input
                ].join(':');
              }
              return [run(false), run(true)].join('|');
            })()
            "#,
        )
        .expect("input type clean-value transition should evaluate");

    assert_eq!(result, "#000000::#ffffff:true|#000000::#ffffff:true");
}

#[test]
fn input_color_value_parses_css_colors_across_value_sources() {
    let mut vm = new_storage_test_vm("https://forms-input-color-css.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const input = document.createElement('input');
              input.type = 'color';
              const values = [
                '#fff',
                'crimson',
                'rgb(1,1,1,0.2)',
                'hsl(150deg 100 53.5)',
                'color(display-p3 .5 0 0)',
                'color(display-p3 1 0 0)',
                'ActiveBorder',
                'currentColor',
                'inherit'
              ].map(value => {
                input.value = value;
                return input.value;
              });

              const fromAttribute = document.createElement('input');
              fromAttribute.setAttribute('type', 'color');
              fromAttribute.setAttribute('value', 'bisque');

              const fromTypeChange = document.createElement('input');
              fromTypeChange.setAttribute('value', 'color(display-p3 .5 0 0)');
              fromTypeChange.type = 'color';

              return [
                values.join(','),
                fromAttribute.value,
                fromTypeChange.value
              ].join('|');
            })()
            "#,
        )
        .expect("CSS color input value sanitization should evaluate");

    assert_eq!(
        result,
        "#ffffff,#dc143c,#010101,#12ff88,#8c0000,#ff0000,#a9a9a9,#000000,#000000|#ffe4c4|#8c0000"
    );
}

#[test]
fn text_control_selection_accessors_parse_webidl_values() {
    let mut vm = new_storage_test_vm("https://forms-selection-accessor-webidl.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const input = document.createElement('input');
              input.type = 'text';
              input.value = 'abcdef';
              let startValueOfCalls = 0;
              let endValueOfCalls = 0;
              function probe(callback) {
                try {
                  return String(callback());
                } catch (error) {
                  return 'throw:' + error.name;
                }
              }
              input.setSelectionRange(1, 5);
              input.selectionStart = {
                valueOf() {
                  startValueOfCalls += 1;
                  return '3.9';
                }
              };
              const afterStartObject = `${input.selectionStart},${input.selectionEnd}:${startValueOfCalls}`;
              input.setSelectionRange(0, 5);
              input.selectionEnd = {
                valueOf() {
                  endValueOfCalls += 1;
                  return null;
                }
              };
              const afterEndObject = `${input.selectionStart},${input.selectionEnd}:${endValueOfCalls}`;
              input.setSelectionRange(1, 4);
              const startSymbol = probe(() => { input.selectionStart = Symbol('start'); });
              const afterStartSymbol = `${input.selectionStart},${input.selectionEnd}`;
              const endThrowing = probe(() => {
                input.selectionEnd = {
                  valueOf() {
                    throw new RangeError('end');
                  }
                };
              });
              const afterEndThrowing = `${input.selectionStart},${input.selectionEnd}`;
              input.selectionStart = -1;
              return [
                afterStartObject,
                afterEndObject,
                startSymbol,
                afterStartSymbol,
                endThrowing,
                afterEndThrowing,
                `${input.selectionStart},${input.selectionEnd}`
              ].join('|');
            })()
            "#,
        )
        .expect("text control selection accessors should parse WebIDL values");

    assert_eq!(
        result,
        "3,5:1|0,0:1|throw:TypeError|1,4|throw:RangeError|1,4|6,6"
    );
}
#[test]
fn input_step_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://forms-step-webidl.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const input = document.createElement('input');
              input.type = 'number';
              input.step = '2';
              input.value = '4';
              function probe(callback) {
                try {
                  return String(callback());
                } catch (error) {
                  return 'throw:' + error.name;
                }
              }
              input.stepUp(undefined);
              const afterDefault = input.value;
              input.stepDown('1.9');
              const afterString = input.value;
              input.stepUp(4294967297);
              const afterWrap = input.value;
              const symbol = probe(() => input.stepUp(Symbol()));
              const afterSymbol = input.value;
              input.stepDown(null);
              return [
                afterDefault,
                afterString,
                afterWrap,
                symbol,
                afterSymbol,
                input.value
              ].join('|');
            })()
            "#,
        )
        .expect("input step methods should parse WebIDL arguments");

    assert_eq!(result, "6|4|6|throw:TypeError|6|6");
}
#[test]
fn input_numeric_setters_parse_webidl_values() {
    let mut vm = new_storage_test_vm("https://forms-numeric-setters-webidl.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const input = document.createElement('input');
              input.type = 'number';
              const textarea = document.createElement('textarea');
              let valueAsNumberCalls = 0;
              let maxLengthCalls = 0;
              let sizeCalls = 0;
              function probe(callback) {
                try {
                  return String(callback());
                } catch (error) {
                  return 'throw:' + error.name;
                }
              }
              input.valueAsNumber = {
                valueOf() {
                  valueAsNumberCalls += 1;
                  return '7.5';
                }
              };
              const valueObject = `${input.value}:${valueAsNumberCalls}`;
              input.valueAsNumber = NaN;
              const valueNaN = `${input.value}:${Number.isNaN(input.valueAsNumber)}`;
              const valueInfinity = probe(() => { input.valueAsNumber = Infinity; });
              const valueSymbol = probe(() => { input.valueAsNumber = Symbol('n'); });
              const valueThrowing = probe(() => {
                input.valueAsNumber = {
                  valueOf() {
                    throw new RangeError('valueAsNumber');
                  }
                };
              });
              const checkbox = document.createElement('input');
              checkbox.type = 'checkbox';
              const unsupportedFinite = probe(() => { checkbox.valueAsNumber = 1; });
              const unsupportedInfinity = probe(() => { checkbox.valueAsNumber = Infinity; });
              const unsupportedSymbol = probe(() => { checkbox.valueAsNumber = Symbol('n'); });
              input.maxLength = {
                valueOf() {
                  maxLengthCalls += 1;
                  return '4.9';
                }
              };
              const maxObject = `${input.maxLength}:${maxLengthCalls}`;
              const maxSymbol = probe(() => { input.maxLength = Symbol('max'); });
              const maxNegative = probe(() => { input.maxLength = -1; });
              textarea.minLength = null;
              const minNull = textarea.minLength;
              input.size = {
                valueOf() {
                  sizeCalls += 1;
                  return '8.9';
                }
              };
              const sizeObject = `${input.size}:${sizeCalls}`;
              const sizeZero = probe(() => { input.size = 0; });
              input.size = -1;
              const sizeNegative = input.size;
              const sizeSymbol = probe(() => { input.size = Symbol('size'); });
              return [
                valueObject,
                valueNaN,
                valueInfinity,
                valueSymbol,
                valueThrowing,
                unsupportedFinite,
                unsupportedInfinity,
                unsupportedSymbol,
                maxObject,
                maxSymbol,
                maxNegative,
                minNull,
                sizeObject,
                sizeZero,
                sizeNegative,
                sizeSymbol
              ].join('|');
            })()
            "#,
        )
        .expect("input numeric setters should parse WebIDL values");

    assert_eq!(
        result,
        "7.5:1|:true|throw:TypeError|throw:TypeError|throw:RangeError|throw:InvalidStateError|throw:TypeError|throw:TypeError|4:1|throw:TypeError|throw:IndexSizeError|0|8:1|throw:IndexSizeError|20|throw:TypeError"
    );
}
#[test]
fn input_value_as_date_matches_supported_temporal_types() {
    let mut vm = new_storage_test_vm("https://forms-value-as-date.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              function probe(callback) {
                try {
                  return String(callback());
                } catch (error) {
                  return 'throw:' + error.name;
                }
              }
              function summarize(type, value, dateValue) {
                const input = document.createElement('input');
                input.type = type;
                input.value = value;
                const millis = input.valueAsDate && input.valueAsDate.getTime();
                input.valueAsDate = new Date(dateValue);
                const afterSet = input.value;
                input.valueAsDate = null;
                return `${type}:${millis}:${afterSet}:${input.value}`;
              }
              const hidden = document.createElement('input');
              hidden.type = 'hidden';
              const date = document.createElement('input');
              date.type = 'date';
              const datetime = document.createElement('input');
              datetime.type = 'datetime-local';
              datetime.value = '2026-01-01T00:00';
              return [
                summarize('date', '2019-12-10', '2016-02-29T00:00:00.000Z'),
                summarize('month', '2019-12', '2016-02-29T00:00:00.000Z'),
                summarize('week', '2019-W50', '2019-12-09T00:00:00.000Z'),
                summarize('time', '12:00', '1970-01-01T23:59:00.000Z'),
                `hidden:${hidden.valueAsDate}:${probe(() => { hidden.valueAsDate = new Date(0); })}`,
                `bad:${probe(() => { date.valueAsDate = {}; })}`,
                `invalid:${probe(() => { date.valueAsDate = new Date(NaN); })}:${date.value}`,
                `datetime:${datetime.valueAsDate}:${probe(() => { datetime.valueAsDate = new Date('2026-01-01T00:00:00.000Z'); })}`
              ].join('|');
            })()
            "#,
        )
        .expect("input valueAsDate should match temporal input semantics");

    assert_eq!(
        result,
        "date:1575936000000:2016-02-29:|month:1575158400000:2016-02:|week:1575849600000:2019-W50:|time:43200000:23:59:|hidden:null:throw:InvalidStateError|bad:throw:TypeError|invalid:undefined:|datetime:null:throw:InvalidStateError"
    );
}
#[test]
fn input_value_as_number_matches_temporal_and_range_types() {
    let mut vm = new_storage_test_vm("https://forms-value-as-number.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              function summarize(type, value, nextNumber) {
                const input = document.createElement('input');
                input.type = type;
                if (type === 'range') {
                  input.min = '0';
                  input.max = '100';
                }
                input.value = value;
                const before = input.valueAsNumber;
                input.valueAsNumber = nextNumber;
                return `${type}:${before}:${input.value}`;
              }
              return [
                summarize('date', '2019-12-10', 1456704000000),
                summarize('month', '2019-12', -1),
                summarize('week', '2019-W50', -20217600000),
                summarize('time', '12:00', 2.7343337071894478e26),
                summarize('time', '23:59', -3600000),
                summarize('datetime-local', '2019-12-10T12:00', 2.7343337071894478e26),
                summarize('number', '123.456', 1000),
                summarize('range', '', 50)
              ].join('|');
            })()
            "#,
        )
        .expect("input valueAsNumber should match HTML input numeric semantics");

    assert_eq!(
        result,
        "date:1575936000000:2016-02-29|month:599:1969-12|week:1575849600000:1969-W20|time:43200000:10:54:10.944|time:86340000:23:00|datetime-local:1575979200000:|number:123.456:1000|range:50:50"
    );
}
#[test]
fn select_collection_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://forms-select-webidl.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const select = document.createElement('select');
              select.innerHTML = `
                <option id="first" name="first-name" value="a">A</option>
                <option id="second" name="second-name" value="b">B</option>
                <option id="null" name="null-name" value="c">C</option>
                <option id="third" value="d">D</option>
              `;
              const options = select.options;
              function probe(callback) {
                try {
                  const value = callback();
                  return value && value.value ? value.value : String(value);
                } catch (error) {
                  return 'throw:' + error.name;
                }
              }
              const itemString = probe(() => select.item('1.9'));
              const itemWrapped = probe(() => select.item(-1));
              const namedNull = probe(() => select.namedItem(null));
              const namedSymbol = probe(() => select.namedItem(Symbol()));
              const itemMissing = probe(() => select.item());
              const namedMissing = probe(() => select.namedItem());
              select.remove('1.9');
              const afterSelectRemove = Array.from(options).map(option => option.value).join('');
              options.remove(Infinity);
              const afterOptionsInfinity = Array.from(options).map(option => option.value).join('');
              options.remove({ valueOf() { return 1; } });
              const afterOptionsObject = Array.from(options).map(option => option.value).join('');
              const optionsBigInt = probe(() => options.remove(1n));
              const selectBigInt = probe(() => select.remove(1n));
              return [
                itemString,
                itemWrapped,
                namedNull,
                namedSymbol,
                itemMissing,
                namedMissing,
                afterSelectRemove,
                afterOptionsInfinity,
                afterOptionsObject,
                optionsBigInt,
                selectBigInt
              ].join('|');
            })()
            "#,
        )
        .expect("select collection methods should parse WebIDL arguments");

    assert_eq!(
        result,
        "b|null|c|throw:TypeError|throw:TypeError|throw:TypeError|acd|cd|c|throw:TypeError|throw:TypeError"
    );
}
#[test]
fn select_remove_overload_preserves_webidl_argument_presence() {
    let mut vm = new_storage_test_vm("https://forms-select-remove-overload.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const parent = document.body || document.documentElement || document;
  const values = select => Array.from(select.options).map(option => option.value).join('');
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const byUndefined = document.createElement('select');
  byUndefined.innerHTML = '<option value="a">A</option><option value="b">B</option>';
  byUndefined.remove(undefined);

  const byNull = document.createElement('select');
  byNull.innerHTML = '<option value="x">X</option><option value="y">Y</option>';
  byNull.remove(null);

  const byOmitted = document.createElement('select');
  byOmitted.innerHTML = '<option value="m">M</option>';
  parent.appendChild(byOmitted);
  const omittedResult = probe(() => byOmitted.remove());

  const bySymbol = document.createElement('select');
  bySymbol.innerHTML = '<option value="q">Q</option>';
  const symbolResult = probe(() => bySymbol.remove(Symbol('index')));

  return [
    values(byUndefined),
    values(byNull),
    omittedResult,
    String(byOmitted.parentNode === null),
    symbolResult,
    values(bySymbol)
  ].join('|');
})()
"##,
        )
        .expect("select.remove overload WebIDL probe should run");

    assert_eq!(result, "b|y|undefined|true|throw:TypeError|q");
}
#[test]
fn select_add_and_collection_setters_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://forms-select-add-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const select = document.createElement('select');
  select.innerHTML = '<option value="a">A</option><option value="b">B</option><option value="c">C</option>';
  const options = select.options;
  const values = [];
  values.push(probe(() => select.add()));
  values.push(probe(() => options.add()));
  values.push(probe(() => select.add(document.createElement('div'))));
  values.push(probe(() => options.add(document.createElement('div'))));
  values.push(probe(() => select.add(new Option('symbol', 'symbol'), Symbol('before'))));
  values.push(probe(() => options.add(new Option('throw', 'throw'), { valueOf() { throw new RangeError('boom'); } })));

  options.add(new Option('middle', 'middle'), '1.9');
  select.add(new Option('front', 'front'), { valueOf() { return 0; } });
  select.add(new Option('end', 'end'), null);
  values.push(Array.from(options).map(option => option.value).join(','));

  const sized = document.createElement('select');
  sized.innerHTML = '<option value="x">X</option><option value="y">Y</option><option value="z">Z</option>';
  sized.length = '1.9';
  values.push(Array.from(sized.options).map(option => option.value).join(','));
  values.push(probe(() => { sized.length = Symbol('length'); }));
  sized.options.length = { valueOf() { return 3; } };
  values.push(String(sized.options.length));
  sized.length = -1;
  values.push(String(sized.length));
  sized.options.length = -1;
  values.push(String(sized.options.length));
  sized.options.length = 100001;
  values.push(String(sized.options.length));
  sized.options.selectedIndex = { valueOf() { return 2; } };
  values.push(String(sized.selectedIndex));
  values.push(probe(() => { sized.options.selectedIndex = Symbol('selectedIndex'); }));
  values.push(String(sized.selectedIndex));

  const indexed = document.createElement('select');
  indexed.options[10001] = new Option('large', 'large');
  values.push(String(indexed.options.length));
  values.push(indexed.options[10001].value);
  indexed.options[10001] = new Option('replace', 'replace');
  values.push(String(indexed.options.length));
  values.push(indexed.options[10001].value);

  return values.join('|');
})()
"#,
        )
        .expect("select add and collection setters should parse WebIDL arguments");

    assert_eq!(
        result,
        "throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:RangeError|front,a,middle,b,c,end|x|throw:TypeError|3|3|3|3|2|throw:TypeError|2|10002|large|10002|replace"
    );
}
