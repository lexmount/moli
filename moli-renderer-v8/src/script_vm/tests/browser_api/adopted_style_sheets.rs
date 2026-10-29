use super::*;

#[test]
fn adopted_stylesheet_setters_convert_iterables_and_preserve_conversion_exceptions() {
    let mut vm = new_parsed_test_vm(
        "https://adopted-stylesheet-iterables.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/adopted-style-sheets-iterables.js"
        ))
        .expect("adopted stylesheet conversion matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "180");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn adopted_stylesheet_assignment_keeps_arrays_and_mutation_methods_in_receiver_realm() {
    let mut vm = new_parsed_test_vm(
        "https://adopted-stylesheet-array-realms.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    assert_eq!(vm.eval(r#"(() => {
      const realms = [window, document.querySelector('#child').contentWindow];
      for (const owner of realms) for (const callee of realms) {
        for (const kind of ['Document', 'ShadowRoot']) {
          const doc = owner.document, host = doc.body.appendChild(doc.createElement('div'));
          const receiver = kind === 'Document' ? doc : host.attachShadow({mode:'open'});
          const own = Object.getOwnPropertyDescriptor(owner[kind].prototype, 'adoptedStyleSheets');
          const borrowed = Object.getOwnPropertyDescriptor(callee[kind].prototype, 'adoptedStyleSheets');
          const sheet = new owner.CSSStyleSheet(), input = new owner.Array(sheet);
          try {
            own.get.call(receiver);
            borrowed.set.call(receiver, input);
            const result = borrowed.get.call(receiver);
            if (result === input || result.length !== 1 || result[0] !== sheet)
              throw Error(kind + ': owned contents');
            if (Object.getPrototypeOf(result) !== owner.Array.prototype)
              throw Error(kind + ': receiver Array realm');
            if (Object.getPrototypeOf(result.push) !== owner.Function.prototype)
              throw Error(kind + ': receiver mutation-method realm');
            if (Object.hasOwn(input, 'push') || input.push !== owner.Array.prototype.push)
              throw Error(kind + ': author input changed');
            input.length = 0;
            if (result.length !== 1) throw Error(kind + ': input container aliases state');
            let caught;
            try {borrowed.set.call(receiver, null);} catch (error) {caught = error;}
            if (Object.getPrototypeOf(caught) !== callee.TypeError.prototype)
              throw Error(kind + ': conversion error realm');
            const marker = {};
            try {
              borrowed.set.call(receiver, {get [Symbol.iterator]() {throw marker;}});
              throw Error(kind + ': accepted abrupt iterator');
            } catch (error) {if (error !== marker) throw error;}
            if (borrowed.get.call(receiver) !== result)
              throw Error(kind + ': failed assignment changed state');
          } finally {own.set.call(receiver, []); host.remove();}
        }
      }
      return true;
    })()"#).unwrap(), "true");
}
