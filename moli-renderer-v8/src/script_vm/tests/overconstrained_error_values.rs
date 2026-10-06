use super::*;

#[test]
fn overconstrained_error_values_preserve_native_metadata_and_receiver_realms() {
    let mut vm = new_parsed_test_vm(
        "https://media-errors.test/",
        "<!doctype html><html><body><iframe></iframe></body></html>",
    );
    assert_eq!(vm.eval(r#"(() => {
      const child = document.querySelector('iframe').contentWindow;
      const check = (ok, label) => { if (!ok) throw Error(label); };
      const error = f => { try { f(); } catch (e) { return e; } throw Error('expected exception'); };
      for (const realm of [window, child]) {
        const C = realm.OverconstrainedError;
        check(typeof C === 'function' && C.length === 1, 'constructor surface');
        const trace = [];
        const e = new C({toString(){trace.push('constraint'); return 'width\ud800';}},
                        {toString(){trace.push('message'); return 'large\udc00';}});
        check(trace.join() === 'constraint,message', 'conversion order');
        check(e instanceof C && e instanceof realm.DOMException && e instanceof realm.Error, 'native inheritance');
        check(e.constraint === 'width\ud800' && e.message === 'large\udc00', 'lossless strings');
        check(!Object.prototype.hasOwnProperty.call(C.prototype, 'message') && !Object.prototype.hasOwnProperty.call(e, 'message'), 'message stays an inherited native attribute');
        check(e.name === 'OverconstrainedError' && e.code === 0 && typeof e.stack === 'string', 'error fields');
        check(Object.getOwnPropertyDescriptor(C.prototype, 'constraint').set === undefined, 'readonly constraint');
        const get = Object.getOwnPropertyDescriptor(C.prototype, 'constraint').get;
        for (const fake of [{}, Object.create(C.prototype), Object.create(e), new Proxy(e,{})]) {
          check(error(() => get.call(fake)) instanceof realm.TypeError, 'native receiver brand');
        }
        check(error(() => C('width')) instanceof realm.TypeError, 'new required');
        check(error(() => new C()) instanceof realm.TypeError, 'required constraint');
        const sentinel = {};
        check(error(() => new C({toString(){throw sentinel;}})) === sentinel, 'exception identity');
        const plain = new C('height');
        check(plain.message === '' && plain.constraint === 'height', 'default message');
      }
      const local = new OverconstrainedError('width');
      const foreignGet = Object.getOwnPropertyDescriptor(child.OverconstrainedError.prototype, 'constraint').get;
      check(foreignGet.call(local) === 'width', 'genuine foreign receiver');
      return true;
    })()"#).unwrap(), "true");
}
