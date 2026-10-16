use super::*;

#[test]
fn native_constructor_entries_defer_newtarget_lookup_and_use_intrinsic_realm_defaults() {
    let mut vm = new_storage_page_task_executor_test_vm("https://constructor-allocation.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const assert = (ok, name) => {if (!ok) throw Error(name)};
        const other = document.querySelector('iframe').contentWindow;
        const log = [];
        const N = new Proxy(function(){}, {get(target,key) {
            if (key === 'prototype') log.push('prototype');
            return Reflect.get(target,key);
        }});
        const init = {get candidate() {log.push('candidate'); return ''},
                      get sdpMid() {log.push('sdpMid'); return '0'}};
        const C = RTCIceCandidate;
        const object = Reflect.construct(C, [init], N);
        assert(log.join() === 'candidate,sdpMid,prototype', 'conversion order: '+log);
        assert(Object.getPrototypeOf(object) === N.prototype, 'subclass prototype');
        log.length = 0;
        const marker = {};
        try {Reflect.construct(C, [{get candidate(){throw marker}}], N)}
        catch(error) {assert(error === marker, 'conversion exception identity')}
        assert(log.length === 0, 'failed conversion skipped prototype lookup');
        const parserPrototype = other.DOMParser.prototype;
        const candidatePrototype = other.RTCIceCandidate.prototype;
        const descriptionPrototype = other.RTCSessionDescription.prototype;
        const F = other.Function(''); F.prototype = null;
        const bound = F.bind(null);
        const proxied = new Proxy(bound, {});
        other.DOMParser = other.RTCIceCandidate = other.RTCSessionDescription = function(){throw Error('poisoned')};
        Reflect.construct = () => {throw Error('poisoned Reflect.construct')};
        assert(Object.getPrototypeOf(new DOMParser()) === DOMParser.prototype, 'captured Reflect.construct');
        const construct = other.Reflect.construct;
        assert(Object.getPrototypeOf(construct(DOMParser, [], proxied)) === parserPrototype, 'parser realm default');
        assert(Object.getPrototypeOf(construct(C, [{sdpMid:'0'}], proxied)) === candidatePrototype, 'candidate realm default');
        assert(Object.getPrototypeOf(construct(RTCSessionDescription, [{type:'offer'}], proxied)) === descriptionPrototype, 'description realm default');
        return true;
    })()"#).unwrap(), "true");
}
