use super::*;

#[test]
fn webrtc_ice_event_constructor_preserves_candidates_and_converts_nullable_urls() {
    let mut vm = new_storage_test_vm("http://webrtc-ice-event.test/");
    let result = vm
        .eval(
            r#"
      (() => {
        const rejects = fn => {
          try { fn(); return false; } catch (error) { return error instanceof TypeError; }
        };
        const candidate = new RTCIceCandidate({sdpMid: 'audio'});
        const rawType = 'ice\ud800\0\udc00';
        const event = new RTCPeerConnectionIceEvent(rawType, {candidate, url: 'turn:\ud800'});
        const checks = [RTCPeerConnectionIceEvent.length === 1,
          rejects(() => new RTCPeerConnectionIceEvent()),
          rejects(() => RTCPeerConnectionIceEvent('ice')),
          rejects(() => new RTCPeerConnectionIceEvent(Symbol())),
          event.type === rawType, event.candidate === candidate, event.url === 'turn:\ufffd',
          new RTCPeerConnectionIceEvent(undefined).type === 'undefined',
          new RTCPeerConnectionIceEvent(null).type === 'null'];
        for (const init of [undefined, null, {}, {candidate: null, url: null},
          {candidate: undefined, url: undefined}]) {
          const empty = new RTCPeerConnectionIceEvent('ice', init);
          checks.push(empty.candidate === null, empty.url === null,
            !empty.bubbles, !empty.cancelable, !empty.composed);
        }
        for (const url of ['', false, 4, 5n, ['not a URL']]) {
          checks.push(new RTCPeerConnectionIceEvent('ice', {url}).url === String(url));
        }
        for (const init of [true, 1, 'init', Symbol(), 1n, {url: Symbol()},
          {candidate: {}}, {candidate: false}, {candidate: 1},
          {candidate: Object.create(candidate)}, {candidate: new Proxy(candidate, {})}]) {
          checks.push(rejects(() => new RTCPeerConnectionIceEvent('ice', init)));
        }
        Object.setPrototypeOf(candidate, null);
        checks.push(new RTCPeerConnectionIceEvent('ice', {candidate}).candidate === candidate);
        return checks.every(Boolean) || JSON.stringify(checks);
      })()
    "#,
        )
        .expect(
            "ICE events should preserve candidate identity and the DOMString/USVString distinction",
        );
    assert_eq!(result, "true");
}

#[test]
fn webrtc_data_channel_event_requires_a_genuine_channel_and_checks_arity_first() {
    let mut vm = new_storage_test_vm("https://webrtc-data-channel-event.test/");
    let result = vm.eval(r#"
      (() => {
        const rejects = fn => {
          try { fn(); return false; } catch (error) { return error instanceof TypeError; }
        };
        const peer = new RTCPeerConnection();
        const channel = peer.createDataChannel('test');
        const event = new RTCDataChannelEvent('channel', {channel});
        let converted = false;
        const checks = [RTCDataChannelEvent.length === 2, event.channel === channel,
          rejects(() => RTCDataChannelEvent('channel', {channel})),
          rejects(() => new RTCDataChannelEvent()),
          rejects(() => new RTCDataChannelEvent({toString() { converted = true; return 'channel'; }})),
          !converted];
        for (const init of [undefined, null, {}, {channel: null}, {channel: undefined},
          false, 1, 'init', Symbol(), 1n, {channel: {}},
          {channel: Object.create(RTCDataChannel.prototype)},
          {channel: Object.create(channel)}, {channel: new Proxy(channel, {})},
          {channel: new RTCIceCandidate({sdpMid: 'audio'})}]) {
          checks.push(rejects(() => new RTCDataChannelEvent('channel', init)));
        }
        Object.defineProperty(RTCDataChannel, Symbol.hasInstance, {
          configurable: true, value() { throw new Error('must not use instanceof'); }
        });
        try {
          checks.push(rejects(() => new RTCDataChannelEvent('channel', {channel: {}})),
            new RTCDataChannelEvent('channel', {channel}).channel === channel);
        } finally { delete RTCDataChannel[Symbol.hasInstance]; }
        Object.setPrototypeOf(channel, null);
        checks.push(new RTCDataChannelEvent('channel', {channel}).channel === channel);
        peer.close();
        return checks.every(Boolean) || JSON.stringify(checks);
      })()
    "#).expect("data channel events should validate arity before conversion and use native interface branding");
    assert_eq!(result, "true");
}

#[test]
fn webrtc_event_dictionary_conversion_reads_inherited_members_first_and_preserves_exceptions() {
    let mut vm = new_storage_test_vm("https://webrtc-event-conversion.test/");
    let result = vm.eval(r#"
      (() => {
        const candidate = new RTCIceCandidate({sdpMid: 'audio'});
        const peer = new RTCPeerConnection();
        const channel = peer.createDataChannel('test');
        const checks = [];
        for (const [Ctor, ownMembers] of [
          [RTCPeerConnectionIceEvent, ['candidate', 'url']], [RTCDataChannelEvent, ['channel']]
        ]) {
          const names = ['bubbles', 'cancelable', 'composed', ...ownMembers];
          const reads = [];
          const dictionary = Object.create({bubbles: [], cancelable: 1, composed: 'yes',
            candidate, channel, url: 'url'});
          const event = new Ctor({toString() { reads.push('type'); return 'converted'; }},
            new Proxy(dictionary, {get(object, name) { reads.push(name); return object[name]; }}));
          checks.push(reads.join(',') === ['type', ...names].join(','),
            event.type === 'converted', event.bubbles, event.cancelable, event.composed);
          const sentinel = new RangeError('original getter failure');
          for (const name of names) {
            const bad = {candidate, channel};
            Object.defineProperty(bad, name, {get() { throw sentinel; }});
            try { new Ctor('event', bad); checks.push(false); }
            catch (error) { checks.push(error === sentinel); }
          }
          try { new Ctor({toString() { throw sentinel; }}, {candidate, channel}); checks.push(false); }
          catch (error) { checks.push(error === sentinel); }
        }
        const sentinel = new RangeError('URL conversion');
        try {
          new RTCPeerConnectionIceEvent('ice', {url: {toString() { throw sentinel; }}});
          checks.push(false);
        } catch (error) { checks.push(error === sentinel); }
        let urlRead = false;
        try {
          new RTCPeerConnectionIceEvent('ice', {candidate: {}, get url() { urlRead = true; return ''; }});
          checks.push(false);
        } catch (error) { checks.push(error instanceof TypeError, !urlRead); }
        Object.defineProperty(Object.prototype, 'bubbles', {
          configurable: true, get() { throw sentinel; }
        });
        try {
          checks.push(new RTCPeerConnectionIceEvent('ice', null).candidate === null);
          try { new RTCDataChannelEvent('channel', null); checks.push(false); }
          catch (error) { checks.push(error instanceof TypeError); }
        } finally { delete Object.prototype.bubbles; }
        peer.close();
        return checks.every(Boolean) || JSON.stringify(checks);
      })()
    "#).expect("WebRTC event dictionaries should convert EventInit first without swallowing getters or exceptions");
    assert_eq!(result, "true");
}

#[test]
fn webrtc_event_members_are_readonly_branded_and_survive_dispatch_and_reinitialization() {
    let mut vm = new_storage_test_vm("https://webrtc-event-dispatch.test/");
    let result = vm.eval(r#"
      (() => {
        const candidate = new RTCIceCandidate({sdpMid: 'audio'});
        const peer = new RTCPeerConnection();
        const channel = peer.createDataChannel('test');
        const checks = [];
        const rejects = fn => {
          try { fn(); return false; } catch (error) { return error instanceof TypeError; }
        };
        for (const [Ctor, members] of [
          [RTCPeerConnectionIceEvent, {candidate, url: 'turn:server'}], [RTCDataChannelEvent, {channel}]
        ]) {
          const event = new Ctor('test', {...members, bubbles: true, cancelable: true, composed: true});
          checks.push(event instanceof Ctor, event instanceof Event,
            Object.getPrototypeOf(Ctor.prototype) === Event.prototype,
            Object.prototype.toString.call(event) === '[object ' + Ctor.name + ']',
            event.isTrusted === false, typeof event.timeStamp === 'number');
          for (const [name, original] of Object.entries(members)) {
            const descriptor = Object.getOwnPropertyDescriptor(Ctor.prototype, name);
            if (!descriptor) return 'missing:' + name;
            checks.push(!Object.hasOwn(event, name), descriptor.enumerable, descriptor.configurable,
              descriptor.get.name === 'get ' + name, descriptor.get.length === 0, !descriptor.set);
            event[name] = 'changed';
            checks.push(event[name] === original, rejects(() => { 'use strict'; event[name] = null; }));
            for (const fake of [undefined, null, {}, Ctor.prototype, new Event('test'),
              Object.create(event), new Proxy(event, {})]) {
              checks.push(rejects(() => descriptor.get.call(fake)));
            }
          }
          const target = new EventTarget();
          let delivered = false;
          target.addEventListener('test', received => {
            delivered = received === event && received.target === target && received.currentTarget === target;
            received.preventDefault();
          });
          checks.push(target.dispatchEvent(event) === false, delivered, event.defaultPrevented,
            event.isTrusted === false, event.currentTarget === null);
          event.initEvent('again', false, false);
          checks.push(event.type === 'again', !event.defaultPrevented, !event.bubbles, !event.cancelable);
          for (const [name, original] of Object.entries(members)) checks.push(event[name] === original);
        }
        peer.close();
        return checks.every(Boolean) || JSON.stringify(checks);
      })()
    "#).expect("WebRTC event attributes should retain native identity through normal Event operations");
    assert_eq!(result, "true");
}

#[test]
fn webrtc_event_constructors_support_subclasses_foreign_members_and_new_target_realms() {
    let mut vm = new_parsed_test_vm(
        "https://webrtc-event-realms.test/",
        "<!doctype html><iframe></iframe>",
    );
    let result = vm.eval(r#"
      (() => {
        const child = document.querySelector('iframe').contentWindow;
        const foreignPeer = new child.RTCPeerConnection();
        const candidate = new child.RTCIceCandidate({sdpMid: 'foreign'});
        const channel = foreignPeer.createDataChannel('foreign');
        const checks = [];
        for (const [name, member, value] of [
          ['RTCPeerConnectionIceEvent', 'candidate', candidate], ['RTCDataChannelEvent', 'channel', channel]
        ]) {
          const Ctor = window[name];
          const init = {[member]: value};
          class Derived extends Ctor {}
          const derived = new Derived('event', init);
          const foreign = new child[name]('event', init);
          const newTarget = child.Function('');
          newTarget.prototype = 0;
          let prototypeReads = 0;
          const target = new Proxy(newTarget, {get(object, name) {
            if (name === 'prototype') prototypeReads++;
            return Reflect.get(object, name);
          }});
          const fallback = Reflect.construct(Ctor, ['event', init], target);
          checks.push(derived instanceof Derived, derived instanceof Ctor,
            derived[member] === value, foreign[member] === value,
            Object.getOwnPropertyDescriptor(Ctor.prototype, member).get.call(foreign) === value,
            Object.getPrototypeOf(fallback) === child[name].prototype, fallback[member] === value,
            prototypeReads === 1);
        }
        foreignPeer.close();
        return checks.every(Boolean) || JSON.stringify(checks);
      })()
    "#).expect("WebRTC event constructors should preserve genuine cross-realm members and NewTarget semantics");
    assert_eq!(result, "true");
}

#[test]
fn webrtc_event_dictionary_snapshots_preserve_frozen_wrapper_state() {
    let mut vm = new_storage_test_vm("https://webrtc-event-snapshot.test/");
    let result = vm.eval(r#"
      (() => {
        const peer = new RTCPeerConnection();
        const candidate = new RTCIceCandidate({sdpMid: 'audio'});
        const channel = peer.createDataChannel('snapshot');
        const error = new RTCError({errorDetail: 'data-channel-failure'});
        const cases = [
          [RTCPeerConnectionIceEvent, 'candidate', candidate],
          [RTCDataChannelEvent, 'channel', channel],
          [RTCErrorEvent, 'error', error]
        ];
        try {
          return cases.every(([Ctor, member, value]) => {
            const event = Object.freeze(new Ctor('before', {
              [member]: value, bubbles: true, cancelable: true, composed: true
            }));
            const stamp = event.timeStamp;
            Event.prototype.initEvent.call(event, 'after\ud800\0', false, true);
            if (event.type !== 'after\ud800\0' || event.bubbles || !event.cancelable ||
                !event.composed || event.timeStamp !== stamp || event[member] !== value) return false;
            const target = new EventTarget();
            let delivered = false;
            target.addEventListener(event.type, received => {
              delivered = received === event && received[member] === value;
              received.preventDefault();
            });
            return target.dispatchEvent(event) === false && delivered && event.defaultPrevented;
          });
        } finally { peer.close(); }
      })()
    "#).expect("freezing a WebRTC event wrapper must not freeze its native event state");
    assert_eq!(result, "true");
}

#[test]
fn webrtc_event_dictionary_snapshots_preserve_registered_native_proxy_payloads() {
    let mut vm = new_storage_test_vm("https://webrtc-event-native-proxy.test/");
    vm.eval(
        r#"
      globalThis.snapshotPeer = new RTCPeerConnection();
      globalThis.snapshotCandidate = new RTCIceCandidate({sdpMid: 'audio'});
      globalThis.snapshotChannel = snapshotPeer.createDataChannel('snapshot');
      globalThis.snapshotError = new RTCError({errorDetail: 'data-channel-failure'});
    "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for name in ["snapshotCandidate", "snapshotChannel", "snapshotError"] {
            let key = crate::util::v8str(scope, name);
            let target =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    let result = vm.eval(r#"
      (() => {
        const cases = [
          [RTCPeerConnectionIceEvent, 'candidate', snapshotCandidate],
          [RTCDataChannelEvent, 'channel', snapshotChannel],
          [RTCErrorEvent, 'error', snapshotError]
        ];
        try {
          return cases.every(([Ctor, member, value]) => {
            let reads = 0, traps = 0;
            const event = new Ctor('snapshot', {
              get [member]() { reads++; return value; }
            });
            if (reads !== 1 || event[member] !== value) return false;
            const wrapped = new Proxy(value, {get() { traps++; throw 42; },
              getPrototypeOf() { traps++; throw 42; }});
            const revoked = Proxy.revocable(value, {}); revoked.revoke();
            for (const invalid of [wrapped, revoked.proxy, Object.create(value)]) {
              try { new Ctor('bad', {[member]: invalid}); return false; }
              catch (error) { if (!(error instanceof TypeError)) return false; }
            }
            return traps === 0 && event[member] === value;
          });
        } finally { snapshotPeer.close(); }
      })()
    "#).expect("snapshot interface leaves retain native proxy identity and reject author wrappers without traps");
    assert_eq!(result, "true");
}
