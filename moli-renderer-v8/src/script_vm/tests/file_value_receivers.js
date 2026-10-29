(() => {
  const failures = [], promises = [];
  const check = (name, fn) => {
    try { if (!fn()) failures.push(name); }
    catch (error) { failures.push(name + ': ' + error); }
  };
  const foreign = document.querySelector('iframe').contentWindow;
  for (const [index, callee] of [window, foreign].entries()) {
    const blob = new foreign.Blob(['contents'], {type: 'text/plain'});
    const file = new foreign.File(['contents'], 'file.txt', {lastModified: 123});
    const transfer = new foreign.DataTransfer(); transfer.items.add(file);
    for (const [name, target, properties] of [
      ['Blob', blob, ['size', 'type']], ['File', file, ['name', 'lastModified']],
      ['FileList', transfer.files, ['length']],
    ]) {
      for (const property of properties) {
        const getter = Object.getOwnPropertyDescriptor(callee[name].prototype, property).get;
        check(name + '/' + property + '/genuine', () => getter.call(target) === target[property]);
        let traps = 0;
        const proxy = new Proxy(target, {get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}});
        const revoked = Proxy.revocable(target, {}); revoked.revoke();
        for (const receiver of [{}, Object.create(target), Object.create(callee[name].prototype), proxy, revoked.proxy]) {
          check(name + '/' + property + '/invalid', () => {
            try {getter.call(receiver);} catch (error) {
              return Object.getPrototypeOf(error) === callee.TypeError.prototype && traps === 0;
            }
            return false;
          });
        }
      }
      if (globalThis.__fileNativeValues) {
        const {target: original, proxy} = __fileNativeValues[name];
        for (const property of properties) {
          const getter = Object.getOwnPropertyDescriptor(callee[name].prototype, property).get;
          check(name + '/' + property + '/native-proxy', () => getter.call(proxy) === original[property]);
        }
      }
    }
    for (const [fn, target] of [
      [callee.Blob.prototype.slice, blob], [callee.FileList.prototype.item, transfer.files],
    ]) {
      let conversions = 0;
      const value = {valueOf() {conversions++; throw 42;}};
      check('method/conversion-before-invalid-receiver/' + index, () => {
        try {fn.call(new Proxy(target, {}), value);} catch (error) {
          return Object.getPrototypeOf(error) === callee.TypeError.prototype && conversions === 0;
        }
        return false;
      });
    }
    for (const method of ['text', 'arrayBuffer', 'bytes']) {
      check(method + '/promise-rejection-boundary/' + index, () => {
        const promise = callee.Blob.prototype[method].call(new Proxy(blob, {}));
        if (Object.getPrototypeOf(promise) !== callee.Promise.prototype) return false;
        promises.push(promise.then(() => failures.push(method + '/resolved'), error => {
          if (Object.getPrototypeOf(error) !== callee.TypeError.prototype) failures.push(method + '/error-realm');
        }));
        return true;
      });
    }
    if (globalThis.__fileNativeValues) {
      const {target, proxy} = __fileNativeValues.Blob;
      promises.push(callee.Blob.prototype.text.call(proxy).then(value => {
        if (value !== 'contents') failures.push('native-proxy/text-content');
      }, error => failures.push('native-proxy/text-error/' + error)));
      check('native-proxy/slice', () => callee.Blob.prototype.slice.call(proxy, 1, 3).size === 2);
      const files = __fileNativeValues.FileList;
      check('native-proxy/item', () => callee.FileList.prototype.item.call(files.proxy, 0) === files.target.item(0));
    }
  }
  globalThis.__fileValueResults = {complete: false, failures};
  Promise.all(promises).then(() => {__fileValueResults.complete = true;});
  return true;
})()
