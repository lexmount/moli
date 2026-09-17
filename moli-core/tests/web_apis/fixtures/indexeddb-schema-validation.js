globalThis.schemaValidationProbe = async function (name) {
  const checks = globalThis.schemaChecks = [];
  const check = (label, actual, expected) => {
    const pass = JSON.stringify(actual) === JSON.stringify(expected);
    checks.push({ label, pass, actual });
  };
  check("createObjectStore.length", IDBDatabase.prototype.createObjectStore.length, 1);
  check("createIndex.length", IDBObjectStore.prototype.createIndex.length, 2);

  await new Promise((resolve, reject) => {
    const request = indexedDB.open(name, 1);
    request.onerror = () => reject(request.error);
    request.onupgradeneeded = () => {
      const order = [];
      const keyPath = ["first", "last"];
      const store = request.result.createObjectStore("records", {
        get keyPath() { order.push("keyPath"); return keyPath; },
        get autoIncrement() { order.push("autoIncrement"); return false; },
      });
      check("object store option getter order", order, ["autoIncrement", "keyPath"]);
      check("stored key path", store.keyPath, keyPath);
      check("stored auto-increment flag", store.autoIncrement, false);
    };
    request.onsuccess = () => {
      request.result.close();
      resolve();
    };
  });
  return { state: checks.every(check => check.pass) ? "pass" : "fail", checks };
};
