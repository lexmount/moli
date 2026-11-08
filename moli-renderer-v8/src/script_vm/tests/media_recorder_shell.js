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
      let constructionError;
      try { Reflect.construct(constructor, []); } catch (error) { constructionError = error; }
      assert(constructionError instanceof realm.TypeError,
        name + ' requires a native MediaStream argument in the callee realm');
      if (realm !== window) assert(!(constructionError instanceof TypeError), name + ' foreign argument TypeError');
      const stream = new realm.MediaStream(), recorder = new constructor(stream);
      assert(recorder.stream === stream && recorder.state === 'inactive' && recorder.mimeType === '', name + ' valid frontend construction');
      let startError;
      try { recorder.start(); } catch (error) { startError = error; }
      assert(startError instanceof realm.DOMException && startError.name === 'NotSupportedError' && startError.code === 9,
        name + ' inactive stream error belongs to the callee realm');
      if (realm !== window) assert(!(startError instanceof DOMException), name + ' foreign DOMException');
      assert(realm[name] === constructor, name + ' materialization remains stable');
    }
  }
  return 'ok';
})()
