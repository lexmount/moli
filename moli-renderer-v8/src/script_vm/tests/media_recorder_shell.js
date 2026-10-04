(() => {
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const interfaces = [["MediaRecorder", "EventTarget", 1]];
  for (const realm of [window, document.getElementById('child').contentWindow]) {
    for (const [name, parentName, length] of interfaces) {
      const constructor = realm[name], parent = realm[parentName ?? 'Object'];
      assert(typeof constructor === 'function' && constructor.name === name && constructor.length === length,
        name + ' interface object');
      const descriptor = Object.getOwnPropertyDescriptor(realm, name);
      assert(descriptor.writable && descriptor.configurable && !descriptor.enumerable, name + ' global descriptor');
      assert(Object.getPrototypeOf(constructor.prototype) === parent.prototype, name + ' prototype inheritance');
      assert(Object.getPrototypeOf(constructor) === (parentName ? parent : realm.Function.prototype), name + ' interface inheritance');
      assert(constructor.prototype.constructor === constructor, name + ' prototype constructor');
      const tag = Object.getOwnPropertyDescriptor(constructor.prototype, Symbol.toStringTag);
      assert(tag.value === name && !tag.writable && !tag.enumerable && tag.configurable, name + ' prototype tag');
      let callError;
      try { constructor(); } catch (error) { callError = error; }
      assert(callError instanceof realm.TypeError, name + ' call requires new in the callee realm');
      if (realm !== window) assert(!(callError instanceof TypeError), name + ' foreign TypeError');
      // These are interface shells. Argument conversion and backend operations remain unavailable.
      let constructionError;
      try { Reflect.construct(constructor, []); } catch (error) { constructionError = error; }
      assert(constructionError instanceof realm.DOMException && constructionError.name === 'NotSupportedError' && constructionError.code === 9,
        name + ' unavailable construction is explicit in the callee realm');
      if (realm !== window) assert(!(constructionError instanceof DOMException), name + ' foreign DOMException');
      assert(realm[name] === constructor, name + ' materialization remains stable');
    }
  }
  return 'ok';
})()
