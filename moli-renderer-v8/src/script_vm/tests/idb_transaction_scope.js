"use strict";

globalThis.runIdbTransactionScopeProbe = async function (kind) {
  const capture = operation => {
    try {
      const value = operation();
      return value !== null && typeof value === "object" ? "returned object" : value;
    }
    catch (error) { return error.name; }
  };
  const units = names => Array.from(names, name =>
    Array.from({length: name.length}, (_, index) => name.charCodeAt(index)));
  const open = (name, version, upgrade) => new Promise((resolve, reject) => {
    const request = indexedDB.open(name, version);
    request.onupgradeneeded = () => upgrade(request.result, request.transaction);
    request.onsuccess = () => resolve(request.result);
    request.onerror = event => {
      event.preventDefault();
      reject(request.error);
    };
  });
  const descriptor = Object.getOwnPropertyDescriptor(
    IDBTransaction.prototype, "objectStoreNames");
  const getter = descriptor && descriptor.get;

  if (kind === "admission") {
    const db = await open("native-scope-admission", 1, db => {
      for (const name of ["", "kept", "outside", "\ud800", "\udc00"])
        db.createObjectStore(name);
    });
    const tx = db.transaction(["kept", "\ud800", ""]);
    globalThis.scopeProbeTransaction = tx;
    const store = tx.objectStore("kept");
    const done = new Promise((resolve, reject) => {
      tx.oncomplete = resolve;
      tx.onabort = () => reject(tx.error || new Error("unexpected abort"));
    });
    const report = {
      repeatedHandle: tx.objectStore("kept") === store,
      emptyName: tx.objectStore("").name === "",
      surrogateName: tx.objectStore("\ud800").name.charCodeAt(0),
      outside: capture(() => tx.objectStore("outside")),
      otherSurrogate: capture(() => tx.objectStore("\udc00")),
      scope: getter ? units(getter.call(tx)) : null
    };
    Object.defineProperty(tx, "objectStoreNames", {
      configurable: true,
      get() { throw new Error("author scope getter"); }
    });
    report.shadowIgnored = capture(() => tx.objectStore("outside"));
    const request = store.get(0);
    request.onerror = () => {};
    tx.commit();
    report.committingHandle = capture(() => tx.objectStore("kept") === store);
    report.committingOutside = capture(() => tx.objectStore("outside"));
    report.committingRequest = capture(() => store.get(0));
    await done;
    report.finishedInside = capture(() => tx.objectStore("kept"));
    report.finishedOutside = capture(() => tx.objectStore("outside"));
    const sentinel = new Error("name conversion sentinel");
    report.conversionBeforeState = false;
    try { tx.objectStore({toString() { throw sentinel; }}); }
    catch (error) { report.conversionBeforeState = error === sentinel; }
    report.retainedScope = getter ? units(getter.call(tx)) : null;
    const aborted = db.transaction("kept");
    aborted.abort();
    report.abortedInside = capture(() => aborted.objectStore("kept"));
    report.abortedOutside = capture(() => aborted.objectStore("outside"));
    db.close();
    return report;
  }

  if (kind === "snapshots") {
    const first = await open("native-scope-snapshots", 1, db => {
      db.createObjectStore("kept");
      db.createObjectStore("\ud800");
    });
    first.close();
    const report = {accessor: !!getter};
    await new Promise((resolve, reject) => {
      const request = indexedDB.open("native-scope-snapshots", 2);
      request.onupgradeneeded = () => {
        const db = request.result;
        const tx = request.transaction;
        const read = () => getter ? getter.call(tx) : tx.objectStoreNames;
        const before = read();
        report.noOwnProperty = !Object.hasOwn(tx, "objectStoreNames");
        report.freshLists = before !== read();
        report.before = units(before);
        Object.defineProperty(tx, "objectStoreNames", {
          value: Object.freeze(["author"]),
          writable: false,
          configurable: false
        });
        db.createObjectStore("created");
        db.deleteObjectStore("kept");
        report.after = units(read());
        report.oldSnapshot = units(before);
        tx.abort();
        report.rollback = units(read());
        report.authorShadowPreserved = tx.objectStoreNames[0] === "author";
        db.close();
      };
      request.onsuccess = () => {
        request.result.close();
        reject(new Error("upgrade should abort"));
      };
      request.onerror = event => {
        event.preventDefault();
        report.error = request.error.name;
        resolve();
      };
    });
    return report;
  }
  throw new Error("unknown probe kind");
};
