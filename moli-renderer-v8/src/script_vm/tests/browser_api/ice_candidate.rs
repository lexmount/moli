use super::*;

#[test]
fn rtc_ice_candidate_has_branded_readonly_attributes_and_four_json_members() {
    let mut vm = new_storage_test_vm("https://ice-candidate.test/");
    let result = vm.eval(r#"
      (() => {
        const candidate = new RTCIceCandidate({sdpMid: 'audio', relayProtocol: 'tls', url: 'turn:example.com'});
        const names = ['candidate', 'sdpMid', 'sdpMLineIndex', 'foundation', 'component',
          'priority', 'address', 'protocol', 'port', 'type', 'tcpType', 'relatedAddress',
          'relatedPort', 'usernameFragment', 'relayProtocol', 'url'];
        const rejects = callback => {
          try { callback(); return false; } catch (error) { return error instanceof TypeError; }
        };
        const checks = [
          RTCIceCandidate.length === 0, candidate instanceof RTCIceCandidate,
          Object.prototype.toString.call(candidate) === '[object RTCIceCandidate]',
          Object.getOwnPropertyNames(candidate).length === 0,
          candidate.toJSON.length === 0,
          rejects(() => RTCIceCandidate({sdpMid: 'audio'})),
          rejects(() => new RTCIceCandidate()), rejects(() => new RTCIceCandidate(null)),
          rejects(() => new RTCIceCandidate({})), rejects(() => new RTCIceCandidate(true)),
          rejects(() => candidate.toJSON.call(Object.create(candidate)))
        ];
        for (const name of names) {
          const descriptor = Object.getOwnPropertyDescriptor(RTCIceCandidate.prototype, name);
          if (!descriptor) return 'missing:' + name;
          checks.push(descriptor.get.name === 'get ' + name, descriptor.get.length === 0,
            descriptor.set === undefined, descriptor.enumerable, descriptor.configurable);
          const value = candidate[name];
          candidate[name] = 'changed';
          checks.push(candidate[name] === value, rejects(() => { 'use strict'; candidate[name] = 'changed'; }));
          for (const fake of [{}, RTCIceCandidate.prototype, Object.create(candidate)]) {
            checks.push(rejects(() => descriptor.get.call(fake)));
          }
        }
        const first = candidate.toJSON();
        const second = candidate.toJSON();
        checks.push(first !== second, JSON.stringify(first) ===
          '{"candidate":"","sdpMid":"audio","sdpMLineIndex":null,"usernameFragment":null}');
        const cloned = new RTCIceCandidate(candidate);
        const signaled = new RTCIceCandidate(first);
        checks.push(cloned.relayProtocol === 'tls', cloned.url === 'turn:example.com',
          signaled.relayProtocol === null, signaled.url === null);
        return checks.every(Boolean);
      })()
    "#).expect("RTCIceCandidate should expose the WebIDL surface without mutable own attributes");
    assert_eq!(result, "true");
}

#[test]
fn rtc_ice_candidate_converts_dictionary_members_in_order_and_preserves_dom_strings() {
    let mut vm = new_storage_test_vm("https://ice-candidate-conversion.test/");
    let result = vm.eval(r#"
      (() => {
        const reads = [];
        const raw = {
          candidate: '\ud800', sdpMid: '\udc00', sdpMLineIndex: -1.9,
          usernameFragment: '\ud800', relayProtocol: 'udp', url: 'turn:\ud800'
        };
        const candidate = new RTCIceCandidate(new Proxy(raw, {
          get(target, name) { reads.push(name); return target[name]; }
        }));
        const checks = [
          reads.join(',') === 'candidate,sdpMLineIndex,sdpMid,usernameFragment,relayProtocol,url',
          candidate.candidate === raw.candidate, candidate.sdpMid === raw.sdpMid,
          candidate.usernameFragment === raw.usernameFragment,
          candidate.sdpMLineIndex === 65535, candidate.url === 'turn:\ufffd',
          new RTCIceCandidate({sdpMLineIndex: 65536}).sdpMLineIndex === 0,
          new RTCIceCandidate({sdpMLineIndex: Infinity}).sdpMLineIndex === 0,
          new RTCIceCandidate({candidate: null, sdpMid: false}).candidate === 'null',
          new RTCIceCandidate({sdpMid: false}).sdpMid === 'false'
        ];
        for (const bad of [{candidate: Symbol()}, {sdpMid: Symbol()}, {sdpMLineIndex: 1n},
          {sdpMLineIndex: Symbol()}, {usernameFragment: Symbol()}, {relayProtocol: 'UDP'}, {url: Symbol()}]) {
          try { new RTCIceCandidate({sdpMid: 'audio', ...bad}); checks.push(false); }
          catch (error) { checks.push(error instanceof TypeError); }
        }
        const error = new RangeError('conversion');
        try { new RTCIceCandidate({candidate: {toString() { throw error; }}, sdpMid: 'audio'}); checks.push(false); }
        catch (caught) { checks.push(caught === error); }
        return checks.every(Boolean);
      })()
    "#).expect("ICE dictionary conversions should preserve WebIDL ordering and original exceptions");
    assert_eq!(result, "true");
}

#[test]
fn rtc_ice_candidate_invalid_strings_preserve_raw_input_without_partial_parsing() {
    let mut vm = new_storage_test_vm("https://ice-candidate-parsing.test/");
    let result = vm.eval(r#"
      (() => {
        const derived = ['foundation', 'component', 'priority', 'address', 'protocol',
          'port', 'type', 'tcpType', 'relatedAddress', 'relatedPort'];
        return ['', 'arbitrary string', 'candidate:x 1 udp 1 127.0.0.1 65536 typ host',
          'candidate:x 1 udp 1 127.0.0.1 9 typ host rport 65536'].every(raw => {
            const candidate = new RTCIceCandidate({candidate: raw, sdpMid: 'video', usernameFragment: 'keep'});
            return candidate.candidate === raw && candidate.usernameFragment === 'keep' &&
              derived.every(name => candidate[name] === null);
          });
      })()
    "#).expect("invalid candidate text should not throw or expose partial derived attributes");
    assert_eq!(result, "true");
}

#[test]
fn rtc_ice_candidate_respects_new_target_and_accepts_genuine_foreign_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://ice-candidate-realms.test/",
        "<!doctype html><iframe></iframe>",
    );
    let result = vm.eval(r#"
      (() => {
        const child = document.querySelector('iframe').contentWindow;
        class Derived extends RTCIceCandidate {}
        const derived = new Derived({sdpMid: 'audio'});
        const foreign = new child.RTCIceCandidate({sdpMid: 'foreign'});
        const newTarget = child.Function('');
        newTarget.prototype = 1;
        const fallback = Reflect.construct(RTCIceCandidate, [{sdpMid: 'fallback'}], newTarget);
        return [
          derived instanceof Derived, derived instanceof RTCIceCandidate,
          Object.getPrototypeOf(foreign) === child.RTCIceCandidate.prototype,
          Object.getPrototypeOf(fallback) === child.RTCIceCandidate.prototype,
          Object.getOwnPropertyDescriptor(RTCIceCandidate.prototype, 'sdpMid').get.call(foreign) === 'foreign',
          RTCIceCandidate.prototype.toJSON.call(foreign).sdpMid === 'foreign'
        ].every(Boolean);
      })()
    "#).expect("ICE constructors should retain subclass and relevant NewTarget prototypes");
    assert_eq!(result, "true");
}
