(() => {
  const child = document.querySelector('iframe').contentWindow;
  const failures = [], promises = [];
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const check = (realm, target, fn, promiseResult = false) => {
    let authorCalls = 0;
    const poison = new Proxy({}, {get() {authorCalls++; throw Error('argument conversion');}});
    const proxy = new Proxy(target, {
      get() {authorCalls++; throw Error('receiver get');},
      getPrototypeOf() {authorCalls++; throw Error('receiver prototype');},
    });
    const revoked = Proxy.revocable(target, {}); revoked.revoke();
    const invalid = [{}, new child.Object(), Object.create(target),
      Object.create(Object.getPrototypeOf(target)), proxy, revoked.proxy];
    const checkError = error => {
      assert(Object.getPrototypeOf(error) === realm.TypeError.prototype, fn.name + ': TypeError realm');
      assert(error.message === 'Illegal invocation', fn.name + ': receiver precedes conversion');
      assert(authorCalls === 0, fn.name + ': receiver check must not execute author code');
    };
    for (const value of invalid) {
      if (promiseResult) {
        const promise = Reflect.apply(fn, value, [poison, poison]);
        assert(Object.getPrototypeOf(promise) === realm.Promise.prototype, fn.name + ': Promise realm');
        promises.push(promise.then(() => {throw Error(fn.name + ': invalid receiver resolved');}, checkError));
      } else {
        let error;
        try {Reflect.apply(fn, value, [poison, poison]);} catch (caught) {error = caught;}
        assert(error, fn.name + ': invalid receiver accepted');
        checkError(error);
      }
    }
  };
  globalThis.receiverChecks = {
    child, assert, check,
    getters(realm, interfaceName, target, names) {
      for (const name of names) {
        const getter = Object.getOwnPropertyDescriptor(realm[interfaceName].prototype, name).get;
        check(realm, target, getter);
        assert(getter.call(target) === target[name], interfaceName + '.' + name + ': foreign receiver');
      }
    },
    finish() {
      Promise.all(promises).then(() => {receiverChecks.complete = true;}, error => {
        failures.push(String(error)); receiverChecks.complete = true;
      });
    },
    failures, complete: false,
  };
})()
