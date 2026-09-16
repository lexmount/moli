globalThis.requestRealmProbe = async function (name = "request-realms") {
  const checks = [];
  const check = (label, pass, actual) => checks.push({label, pass, actual});
  const frame = document.createElement("iframe");
  (document.body || document.documentElement).appendChild(frame);
  const child = frame.contentWindow;
  const pending = [];
  try {
    for (const [label, owner, callee] of [["main", globalThis, child], ["child", child, globalThis]]) {
      const request = owner.indexedDB.open(`${name}-${label}`, 1);
      const getters = Object.fromEntries(["source", "transaction", "readyState", "result", "error"].map(key =>
        [key, Object.getOwnPropertyDescriptor(callee.IDBRequest.prototype, key).get]));
      check(`${label}:source`, getters.source.call(request) === null, "null");
      check(`${label}:transaction`, getters.transaction.call(request) === null, "null");
      check(`${label}:state`, getters.readyState.call(request) === "pending", getters.readyState.call(request));
      for (const key of ["result", "error"]) {
        try {
          getters[key].call(request);
          check(`${label}:${key}:pending-realm`, false, "accepted");
        } catch (error) {
          check(`${label}:${key}:pending-realm`, error.name === "InvalidStateError" &&
            error instanceof callee.DOMException && !(error instanceof owner.DOMException), error.name);
        }
      }
      for (const key of Object.keys(getters)) {
        try {
          getters[key].call(new Proxy(request, {}));
          check(`${label}:${key}:brand-realm`, false, "accepted");
        } catch (error) {
          check(`${label}:${key}:brand-realm`, error instanceof callee.TypeError &&
            !(error instanceof owner.TypeError), error.name);
        }
      }
      pending.push(new Promise((resolve, reject) => {
        request.onerror = () => reject(request.error);
        request.onupgradeneeded = () => {
          try {
            check(`${label}:upgrade-result`, getters.result.call(request) === request.result &&
              request.result instanceof owner.IDBDatabase && !(request.result instanceof callee.IDBDatabase),
              request.readyState);
            check(`${label}:upgrade-transaction`, getters.transaction.call(request) === request.transaction &&
              request.transaction instanceof owner.IDBTransaction, request.transaction.mode);
          } catch (error) { reject(error); }
        };
        request.onsuccess = () => {
          try {
            check(`${label}:success`, getters.readyState.call(request) === "done" &&
              getters.error.call(request) === null && getters.transaction.call(request) === null &&
              getters.result.call(request) === request.result, request.readyState);
            request.result.close();
            resolve();
          } catch (error) { reject(error); }
        };
      }));
    }
    await Promise.all(pending);
    return {state: checks.every(check => check.pass) ? "pass" : "fail", checks};
  } finally {
    frame.remove();
  }
};
