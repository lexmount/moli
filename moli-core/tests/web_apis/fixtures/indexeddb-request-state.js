globalThis.requestStateProbe = async function (name = "request-state") {
  const checks = globalThis.requestStateChecks = [];
  const check = (label, pass, actual) => checks.push({ label, pass, actual });
  const NativeEvent = Event;
  const VersionChangeEvent = IDBVersionChangeEvent;
  const attributes = ["result", "error", "source", "transaction", "readyState"];
  const descriptors = Object.fromEntries(attributes.map(key =>
    [key, Object.getOwnPropertyDescriptor(IDBRequest.prototype, key)]));
  const read = (request, key) => descriptors[key]?.get
    ? descriptors[key].get.call(request) : request[key];
  const throws = (label, constructor, errorName, operation) => {
    try {
      operation();
      check(label, false, "accepted");
    } catch (error) {
      check(label, error instanceof constructor && error.name === errorName, error.name);
    }
  };
  const pending = (label, request) => {
    check(`${label}:state`, read(request, "readyState") === "pending", read(request, "readyState"));
    for (const key of ["result", "error"]) {
      throws(`${label}:${key}`, DOMException, "InvalidStateError", () => read(request, key));
    }
  };
  const eventCheck = (label, event, target, version = null, error = false) => {
    check(`${label}:target`, event.target === target && event.currentTarget === target,
      event.eventPhase);
    check(`${label}:class`, event instanceof NativeEvent &&
      (version === null ? !(event instanceof VersionChangeEvent) : event instanceof VersionChangeEvent),
      Object.prototype.toString.call(event));
    check(`${label}:flags`, event.isTrusted && event.bubbles === error &&
      event.cancelable === error && !event.composed, [event.isTrusted, event.bubbles, event.cancelable]);
    if (version !== null) {
      check(`${label}:versions`, event.oldVersion === version[0] && event.newVersion === version[1],
        [event.oldVersion, event.newVersion]);
    }
  };
  const connections = new Set();
  const guarded = (reject, callback) => (...args) => {
    try { callback(...args); } catch (error) { reject(error); }
  };
  const openDatabase = (version, upgrade = () => {}) => new Promise((resolve, reject) => {
    const request = indexedDB.open(name, version);
    pending(`open-${version}`, request);
    request.onerror = () => reject(request.error);
    request.onupgradeneeded = guarded(reject, event => {
      connections.add(request.result);
      upgrade(request, event, reject);
    });
    request.onsuccess = guarded(reject, event => {
      connections.add(request.result);
      eventCheck(`open-${version}:success`, event, request);
      check(`open-${version}:done`, request.readyState === "done" && request.error === null &&
        request.transaction === null && request.source === null, request.readyState);
      resolve(request.result);
    });
  });
  try {
    for (const key of attributes) {
      const descriptor = descriptors[key];
      check(`descriptor:${key}`, typeof descriptor?.get === "function" &&
        descriptor.set === undefined && descriptor.enumerable && descriptor.configurable,
        Object.keys(descriptor || {}));
    }
    check("author-event:untrusted", !new NativeEvent("success").isTrusted &&
      !new VersionChangeEvent("success").isTrusted, "author events");
    // Native delivery must use intrinsic constructors, including in workers.
    globalThis.Event = globalThis.IDBVersionChangeEvent = function () {
      throw new Error("author constructor must not be used for IndexedDB delivery");
    };
    const database = await openDatabase(3, (request, event, reject) => {
      eventCheck("upgrade", event, request, [0, 3]);
      const db = request.result;
      const tx = request.transaction;
      check("upgrade:done", request.readyState === "done" && request.error === null, request.readyState);
      const revoked = Proxy.revocable(request, {});
      revoked.revoke();
      for (const key of attributes) {
        check(`attribute:${key}:inherited`, !Object.hasOwn(request, key), Object.keys(request));
        check(`attribute:${key}:readonly`, descriptors[key]?.get && !Reflect.set(request, key, 17), key);
        for (const [label, value] of [
          ["null", null], ["plain", {}], ["inherited", Object.create(request)],
          ["proxy", new Proxy(request, {})], ["revoked", revoked.proxy],
        ]) {
          if (descriptors[key]?.get) {
            throws(`brand:${key}:${label}`, TypeError, "TypeError", () => descriptors[key].get.call(value));
          } else {
            check(`brand:${key}:${label}`, false, "missing getter");
          }
        }
      }
      const store = db.createObjectStore("records");
      store.put("one", 1);
      store.put("two", 2);
      tx.oncomplete = guarded(reject, complete => {
        eventCheck("upgrade:complete", complete, tx);
        check("upgrade:complete-state", request.readyState === "done" && request.result === db &&
          request.transaction === tx && request.error === null, request.readyState);
      });
    });

    await new Promise((resolve, reject) => {
      const tx = database.transaction("records");
      tx.onabort = () => reject(tx.error || new Error("read transaction aborted"));
      tx.oncomplete = resolve;
      const store = tx.objectStore("records");
      const request = store.get(1);
      pending("get", request);
      check("get:source-transaction", request.source === store && request.transaction === tx, request.readyState);
      // Shadowing public attributes cannot interfere with internal state or task delivery.
      for (const key of attributes) {
        Object.defineProperty(request, key, {
          configurable: true,
          get() { throw new Error(`author ${key} getter`); },
          set() { throw new Error(`author ${key} setter`); },
        });
      }
      request.onerror = () => reject(read(request, "error"));
      request.onsuccess = guarded(reject, event => {
        eventCheck("get:success", event, request);
        check("get:result", read(request, "readyState") === "done" && read(request, "result") === "one" &&
          read(request, "error") === null, read(request, "result"));
        check("get:identity", read(request, "source") === store && read(request, "transaction") === tx, "native state");
      });
      const cursorRequest = store.openCursor();
      let nextKey = 1;
      pending("cursor:initial", cursorRequest);
      cursorRequest.onerror = () => reject(cursorRequest.error);
      cursorRequest.onsuccess = guarded(reject, () => {
        const cursor = cursorRequest.result;
        check(`cursor:${nextKey}:done`, cursorRequest.readyState === "done" && cursorRequest.error === null,
          cursorRequest.readyState);
        if (cursor) {
          check(`cursor:${nextKey}:value`, cursor.key === nextKey && cursor.request === cursorRequest, cursor.key);
          cursor.continue();
          pending(`cursor:${nextKey}:continue`, cursorRequest);
          ++nextKey;
        } else {
          check("cursor:exhausted", nextKey === 3 && cursor === null, nextKey);
        }
      });
    });

    await new Promise((resolve, reject) => {
      const tx = database.transaction("records", "readwrite");
      const request = tx.objectStore("records").add("duplicate", 1);
      pending("add-error", request);
      request.onsuccess = () => reject(new Error("duplicate key accepted"));
      tx.onabort = () => reject(tx.error || new Error("canceled request error aborted transaction"));
      tx.oncomplete = resolve;
      request.onerror = guarded(reject, event => {
        eventCheck("add-error:event", event, request, null, true);
        check("add-error:state", request.readyState === "done" && request.result === undefined &&
          request.error.name === "ConstraintError", request.error.name);
        event.preventDefault();
        check("add-error:canceled", event.defaultPrevented, event.defaultPrevented);
      });
    });

    const upgraded = await new Promise((resolve, reject) => {
      const request = indexedDB.open(name, 4);
      pending("blocked:initial", request);
      database.onversionchange = guarded(reject, event => {
        eventCheck("versionchange", event, database, [3, 4]);
      });
      request.onblocked = guarded(reject, event => {
        eventCheck("blocked", event, request, [3, 4]);
        pending("blocked", request);
        check("blocked:references", request.source === null && request.transaction === null, "null");
        database.close();
      });
      request.onupgradeneeded = guarded(reject, event => {
        connections.add(request.result);
        eventCheck("blocked:upgrade", event, request, [3, 4]);
        check("blocked:upgrade-state", request.readyState === "done" && request.error === null &&
          request.transaction instanceof IDBTransaction, request.readyState);
      });
      request.onerror = () => reject(request.error);
      request.onsuccess = guarded(reject, () => resolve(request.result));
    });
    upgraded.close();
    const reopened = await openDatabase(4, () => { throw new Error("same-version open upgraded"); });
    reopened.close();

    await new Promise((resolve, reject) => {
      const request = indexedDB.open(name, 5);
      request.onupgradeneeded = guarded(reject, () => {
        const db = request.result;
        connections.add(db);
        const tx = request.transaction;
        tx.onabort = () => reject(new Error("close after commit aborted upgrade"));
        tx.oncomplete = guarded(reject, () => {
          db.close();
          check("close-after-commit:state", request.readyState === "done" &&
            request.result === db && request.error === null && request.transaction === tx,
            request.readyState);
        });
      });
      request.onsuccess = () => reject(new Error("closed connection open succeeded"));
      request.onerror = guarded(reject, event => {
        eventCheck("close-after-commit:error", event, request, null, true);
        check("close-after-commit:final-state", request.readyState === "done" &&
          request.result === undefined && request.error.name === "AbortError" &&
          request.transaction === null, request.error.name);
        event.preventDefault();
        resolve();
      });
    });

    await new Promise((resolve, reject) => {
      const request = indexedDB.open(name, 6);
      request.onupgradeneeded = guarded(reject, () => {
        const db = request.result;
        connections.add(db);
        const tx = request.transaction;
        tx.oncomplete = () => reject(new Error("aborted upgrade completed"));
        tx.onabort = guarded(reject, event => {
          check("abort:event", event.isTrusted && event.bubbles && !event.cancelable, event.type);
          check("abort:open-state", request.readyState === "done" && request.result === db &&
            request.error === null && request.transaction === tx, request.readyState);
        });
        tx.abort();
        check("abort:immediate-state", request.readyState === "done" && request.error === null,
          request.readyState);
      });
      request.onsuccess = () => reject(new Error("aborted open succeeded"));
      request.onerror = guarded(reject, event => {
        eventCheck("abort:open-error", event, request, null, true);
        check("abort:final-state", request.readyState === "done" && request.result === undefined &&
          request.error.name === "AbortError" && request.transaction === null, request.error.name);
        event.preventDefault();
        resolve();
      });
    });

    await new Promise((resolve, reject) => {
      const request = indexedDB.open(name, 2);
      pending("version-error", request);
      request.onsuccess = () => reject(new Error("version downgrade succeeded"));
      request.onerror = guarded(reject, event => {
        eventCheck("version-error:event", event, request, null, true);
        check("version-error:state", request.readyState === "done" && request.result === undefined &&
          request.error.name === "VersionError" && request.transaction === null, request.error.name);
        event.preventDefault();
        check("version-error:canceled", event.defaultPrevented, event.defaultPrevented);
        resolve();
      });
    });

    const deleted = (label, request, version) => new Promise((resolve, reject) => {
      pending(label, request);
      request.onerror = () => reject(request.error);
      request.onsuccess = guarded(reject, event => {
        eventCheck(label, event, request, [version, null]);
        check(`${label}:done`, request.readyState === "done" && request.result === undefined &&
          request.error === null && request.source === null && request.transaction === null,
          request.readyState);
        resolve();
      });
    });
    const first = deleted("delete:existing", indexedDB.deleteDatabase(name), 5);
    const second = deleted("delete:missing", indexedDB.deleteDatabase(name), 0);
    const recreate = openDatabase(9).then(db => db.close());
    const third = deleted("delete:recreated", indexedDB.deleteDatabase(name), 9);
    await Promise.all([first, second, recreate, third]);
    return { state: checks.every(result => result.pass) ? "pass" : "fail", checks };
  } finally {
    globalThis.Event = NativeEvent;
    globalThis.IDBVersionChangeEvent = VersionChangeEvent;
    for (const connection of connections) connection.close();
  }
};
