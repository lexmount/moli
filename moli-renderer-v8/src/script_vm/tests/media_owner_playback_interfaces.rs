use super::*;

#[test]
fn media_owner_playback_interfaces_preserve_native_inheritance_and_realm_contracts() {
    for url in [
        "https://media-owner-playback-interfaces.test/",
        "http://media-owner-playback-interfaces.test/",
        "http://localhost/",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm(url);
        vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
            .unwrap();
        let result = vm
            .eval(
                r#"(() => {
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const interfaces = [
    ['MediaStreamTrack', 'EventTarget'],
    ['CanvasCaptureMediaStreamTrack', 'MediaStreamTrack'],
    ['MediaSession', 'Object'],
    ['TimeRanges', 'Object'],
    ['VideoPlaybackQuality', 'Object']
  ];
  for (const realm of [window, document.getElementById('child').contentWindow]) {
    for (const [name, parentName] of interfaces) {
      const constructor = realm[name], parent = realm[parentName];
      assert(typeof constructor === 'function' && constructor.name === name && constructor.length === 0,
        name + ' interface object');
      const descriptor = Object.getOwnPropertyDescriptor(realm, name);
      assert(descriptor.writable && descriptor.configurable && !descriptor.enumerable,
        name + ' global descriptor');
      assert(Object.getPrototypeOf(constructor.prototype) === parent.prototype,
        name + ' prototype inheritance');
      assert(Object.getPrototypeOf(constructor) ===
        (parentName === 'Object' ? realm.Function.prototype : parent),
        name + ' interface inheritance');
      assert(constructor.prototype.constructor === constructor, name + ' prototype constructor');
      const tag = Object.getOwnPropertyDescriptor(constructor.prototype, Symbol.toStringTag);
      assert(tag.value === name && !tag.writable && !tag.enumerable && tag.configurable,
        name + ' prototype tag');
      for (const call of [() => constructor(), () => new constructor()]) {
        let error;
        try { call(); } catch (caught) { error = caught; }
        assert(error instanceof realm.TypeError,
          name + ' rejects direct construction in the callee realm');
        if (realm !== window) assert(!(error instanceof TypeError), name + ' foreign error realm');
      }
    }
  }
  return 'ok';
})()"#,
            )
            .unwrap();
        assert_eq!(result, "ok", "{url}");
    }
}
