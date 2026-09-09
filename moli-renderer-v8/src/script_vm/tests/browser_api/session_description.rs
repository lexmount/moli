use super::*;

#[test]
fn rtc_session_description_requires_a_dictionary_with_a_valid_type() {
    let mut vm = new_storage_test_vm("http://session-description.test/");
    let result = vm.eval(r#"
      (() => {
        const rejects = callback => {
          try { callback(); return false; } catch (error) { return error instanceof TypeError; }
        };
        const checks = [
          RTCSessionDescription.length === 1,
          RTCSessionDescription.name === 'RTCSessionDescription',
          rejects(() => RTCSessionDescription({type: 'offer'})),
          rejects(() => new RTCSessionDescription())
        ];
        for (const value of [undefined, null, {}, true, 3, 'offer', Symbol(), 1n,
          {type: undefined}, {type: null}, {type: ''}, {type: 'Offer'},
          {type: ' offer'}, {type: 'answer\0'}, {type: Symbol()}, {type: 1}]) {
          checks.push(rejects(() => new RTCSessionDescription(value)));
        }
        for (const type of ['offer', 'pranswer', 'answer', 'rollback']) {
          const description = new RTCSessionDescription({type});
          checks.push(description.type === type, description.sdp === '');
          const raw = 'not valid SDP\r\n\ud800\0\udc00';
          const supplied = new RTCSessionDescription({type, sdp: raw});
          checks.push(supplied.sdp === raw, supplied.toJSON().sdp === raw);
        }
        const values = [undefined, null, false, 123, 1n, ['sdp']];
        for (const sdp of values) {
          checks.push(new RTCSessionDescription({type: 'offer', sdp}).sdp ===
            (sdp === undefined ? '' : String(sdp)));
        }
        checks.push(rejects(() => new RTCSessionDescription({type: 'offer', sdp: Symbol()})));
        return checks.every(Boolean);
      })()
    "#).expect("RTCSessionDescription should require an RTCSdpType and retain DOMString SDP without validation");
    assert_eq!(result, "true");
}

#[test]
fn rtc_session_description_converts_members_in_order_and_preserves_exceptions() {
    let mut vm = new_storage_test_vm("https://session-description-conversion.test/");
    let result = vm.eval(r#"
      (() => {
        const reads = [];
        const dictionary = Object.create({
          sdp: {toString() { reads.push('convert sdp'); return 'sdp'; }},
          type: {toString() { reads.push('convert type'); return 'answer'; }}
        });
        const description = new RTCSessionDescription(new Proxy(dictionary, {
          get(target, name) { reads.push(name); return target[name]; }
        }));
        const checks = [reads.join(',') === 'sdp,convert sdp,type,convert type',
          description.type === 'answer', description.sdp === 'sdp'];
        const error = new RangeError('original exception');
        for (const member of ['sdp', 'type']) {
          for (const conversion of [false, true]) {
            const bad = {type: 'offer'};
            Object.defineProperty(bad, member, {
              get() { if (!conversion) throw error; return {toString() { throw error; }}; }
            });
            try { new RTCSessionDescription(bad); checks.push(false); }
            catch (caught) { checks.push(caught === error); }
          }
        }
        let typeRead = false;
        try {
          new RTCSessionDescription({sdp: Symbol(), get type() { typeRead = true; return 'offer'; }});
          checks.push(false);
        } catch (caught) { checks.push(caught instanceof TypeError, !typeRead); }
        let sdpRead = false;
        try { new RTCSessionDescription({get sdp() { sdpRead = true; return ''; }}); checks.push(false); }
        catch (caught) { checks.push(caught instanceof TypeError, sdpRead); }
        Object.defineProperty(Object.prototype, 'type', {
          configurable: true, get() { throw error; }
        });
        try {
          for (const value of [undefined, null]) {
            try { new RTCSessionDescription(value); checks.push(false); }
            catch (caught) { checks.push(caught instanceof TypeError); }
          }
        } finally { delete Object.prototype.type; }
        const clone = new RTCSessionDescription(description);
        checks.push(clone !== description, clone.type === 'answer', clone.sdp === 'sdp');
        return checks.every(Boolean);
      })()
    "#).expect("RTCSessionDescription dictionary conversion should observe WebIDL order and original exceptions");
    assert_eq!(result, "true");
}

#[test]
fn rtc_session_description_has_readonly_branded_attributes_and_default_json() {
    let mut vm = new_storage_test_vm("https://session-description-prototype.test/");
    let result = vm.eval(r#"
      (() => {
        const description = new RTCSessionDescription({type: 'offer', sdp: 'original'});
        const rejects = callback => {
          try { callback(); return false; } catch (error) { return error instanceof TypeError; }
        };
        const checks = [description instanceof RTCSessionDescription,
          Object.prototype.toString.call(description) === '[object RTCSessionDescription]',
          Object.getOwnPropertyNames(description).length === 0,
          Object.getPrototypeOf(RTCSessionDescription.prototype) === Object.prototype];
        const fakes = [undefined, null, {}, RTCSessionDescription.prototype,
          Object.create(description), new Proxy(description, {})];
        for (const name of ['type', 'sdp']) {
          const descriptor = Object.getOwnPropertyDescriptor(RTCSessionDescription.prototype, name);
          if (!descriptor) return 'missing:' + name;
          checks.push(descriptor.get.name === 'get ' + name, descriptor.get.length === 0,
            descriptor.set === undefined, descriptor.enumerable, descriptor.configurable);
          const original = description[name];
          description[name] = 'changed';
          checks.push(description[name] === original,
            rejects(() => { 'use strict'; description[name] = 'changed'; }));
          for (const fake of fakes) checks.push(rejects(() => descriptor.get.call(fake)));
        }
        const method = Object.getOwnPropertyDescriptor(RTCSessionDescription.prototype, 'toJSON');
        checks.push(method.value.name === 'toJSON', method.value.length === 0,
          method.writable, method.enumerable, method.configurable);
        for (const fake of fakes) checks.push(rejects(() => method.value.call(fake)));
        const first = description.toJSON();
        const second = description.toJSON();
        checks.push(first !== second, Object.getPrototypeOf(first) === Object.prototype,
          Object.keys(first).join(',') === 'type,sdp',
          JSON.stringify(description) === '{"type":"offer","sdp":"original"}');
        for (const name of ['type', 'sdp']) {
          const descriptor = Object.getOwnPropertyDescriptor(first, name);
          checks.push(descriptor.writable, descriptor.enumerable, descriptor.configurable);
          first[name] = 'changed';
          Object.defineProperty(description, name, {get() { throw new Error('shadow getter'); }});
        }
        checks.push(JSON.stringify(description.toJSON()) === '{"type":"offer","sdp":"original"}');
        return checks.every(Boolean);
      })()
    "#).expect("RTCSessionDescription should expose readonly IDL attributes and serialize only internal values");
    assert_eq!(result, "true");
}

#[test]
fn rtc_session_description_preserves_subclasses_and_cross_realm_behavior() {
    let mut vm = new_parsed_test_vm(
        "https://session-description-realms.test/",
        "<!doctype html><iframe></iframe>",
    );
    let result = vm.eval(r#"
      (() => {
        const child = document.querySelector('iframe').contentWindow;
        class Derived extends RTCSessionDescription {}
        const derived = new Derived({type: 'offer'});
        const foreign = new child.RTCSessionDescription({type: 'answer', sdp: 'foreign'});
        const newTarget = child.Function('');
        newTarget.prototype = null;
        const fallback = Reflect.construct(RTCSessionDescription, [{type: 'rollback'}], newTarget);
        const json = RTCSessionDescription.prototype.toJSON.call(foreign);
        const foreignJson = child.RTCSessionDescription.prototype.toJSON.call(derived);
        return [derived instanceof Derived, derived instanceof RTCSessionDescription,
          Object.getPrototypeOf(foreign) === child.RTCSessionDescription.prototype,
          Object.getPrototypeOf(fallback) === child.RTCSessionDescription.prototype,
          Object.getOwnPropertyDescriptor(RTCSessionDescription.prototype, 'sdp').get.call(foreign) === 'foreign',
          json.type === 'answer', json.sdp === 'foreign',
          Object.getPrototypeOf(json) === Object.prototype,
          Object.getPrototypeOf(foreignJson) === child.Object.prototype
        ].every(Boolean);
      })()
    "#).expect("RTCSessionDescription should use NewTarget prototypes and the toJSON method's realm");
    assert_eq!(result, "true");
}
