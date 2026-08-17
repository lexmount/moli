use super::*;
use moli_url::origin_ascii_serialization;

#[test]
fn indexed_db_runtime_state_is_created_on_first_use_without_window_slots() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-lazy-runtime.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const slots = [
    "__moliWindowIndexedDB",
    "__moliIndexedDbTaskQueue",
    "__moliIndexedDbOpenDatabases",
    "__moliIndexedDbBlockedOpenQueue",
    "__moliIndexedDbReadwriteTransactionQueue"
  ];
  const before = slots.map((name) => Object.prototype.hasOwnProperty.call(window, name));
  const descriptor = Object.getOwnPropertyDescriptor(window, "indexedDB");
  const factory = indexedDB;
  const after = slots.map((name) => Object.prototype.hasOwnProperty.call(window, name));
  return JSON.stringify({
    before,
    after,
    descriptor: {
      present: !!descriptor,
      enumerable: !!descriptor?.enumerable,
      configurable: !!descriptor?.configurable,
      hasGetter: typeof descriptor?.get === "function"
    },
    constructors: [typeof IDBFactory, typeof IDBRequest, typeof IDBDatabase],
    factory: {
      type: typeof factory,
      open: typeof factory.open,
      branded: factory instanceof IDBFactory
    }
  });
})()
"#,
        )
        .expect("indexeddb runtime state probe should evaluate");

    assert_eq!(
        result,
        r#"{"before":[false,false,false,false,false],"after":[false,false,false,false,false],"descriptor":{"present":true,"enumerable":true,"configurable":true,"hasGetter":true},"constructors":["function","function","function"],"factory":{"type":"object","open":"function","branded":true}}"#
    );
}

#[test]
fn indexed_db_ignores_legacy_window_slot_names() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-legacy-slot-poison.test/");

    vm.eval(
        r#"
(() => {
  const poisonFactory = { open: () => { throw new Error("poisoned factory"); } };
  window.__moliWindowIndexedDB = poisonFactory;
  window.__moliIndexedDbTaskQueue = [{ __poisoned: "task" }];
  window.__moliIndexedDbOpenDatabases = [{ __poisoned: "open" }];
  window.__moliIndexedDbBlockedOpenQueue = [{ __poisoned: "blocked" }];
  window.__moliIndexedDbReadwriteTransactionQueue = [{ __poisoned: "readwrite" }];

  const factory = indexedDB;
  globalThis.__indexedDbLegacySlotResult = "pending";
  const open = factory.open("app", 1);
  open.onerror = () => {
    globalThis.__indexedDbLegacySlotResult = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
    globalThis.__indexedDbLegacySlotResult = JSON.stringify({
      ignoredFactory: factory !== poisonFactory,
      sameFactory: indexedDB === factory,
      brandedFactory: factory instanceof IDBFactory,
      brandedDatabase: open.result instanceof IDBDatabase,
      poisonedTaskSlotStillPageOwned: window.__moliIndexedDbTaskQueue?.[0]?.__poisoned === "task"
    });
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb legacy slot poison workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbLegacySlotResult)")
        .expect("indexeddb legacy slot poison result should be readable");

    assert_eq!(
        result,
        r#"{"ignoredFactory":true,"sameFactory":true,"brandedFactory":true,"brandedDatabase":true,"poisonedTaskSlotStillPageOwned":true}"#
    );
}

#[tokio::test]
async fn indexed_db_databases_returns_committed_name_version_snapshot() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://indexeddb-databases-list.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbDatabasesResult = "pending";
  function openDb(name, version) {
    return new Promise((resolve, reject) => {
      const request = indexedDB.open(name, version);
      request.onupgradeneeded = () => {};
      request.onerror = () => reject(request.error && request.error.name);
      request.onsuccess = () => {
        request.result.close();
        resolve();
      };
    });
  }
  Promise.resolve()
    .then(() => openDb("beta", 2))
    .then(() => openDb("alpha", 1))
    .then(() => {
      const setterHits = [];
      for (const key of ["name", "version"]) {
        Object.defineProperty(Object.prototype, key, {
          configurable: true,
          set(value) { setterHits.push(`${key}:${typeof value}`); }
        });
      }
      return indexedDB.databases().finally(() => {
        for (const key of ["name", "version"]) {
          delete Object.prototype[key];
        }
      }).then(databases => ({ databases, setterHits }));
    })
    .then(({ databases, setterHits }) => {
      const firstNameDescriptor = Object.getOwnPropertyDescriptor(databases[0], "name");
      const firstVersionDescriptor = Object.getOwnPropertyDescriptor(databases[0], "version");
      globalThis.__indexedDbDatabasesResult = JSON.stringify({
        promiseBrand: indexedDB.databases() instanceof Promise,
        entries: databases.map(database => `${database.name}:${database.version}`),
        objectKeys: databases.map(database => Object.keys(database).join(",")),
        firstNameDescriptor,
        firstVersionDescriptor,
        setterHits
      });
    }, error => {
      globalThis.__indexedDbDatabasesResult = `rejected:${error && error.name}`;
    });
  return "scheduled";
})()
"#,
    )
    .expect("indexedDB databases list workflow should schedule");

    for _ in 0..8 {
        if vm
            .eval("String(globalThis.__indexedDbDatabasesResult !== 'pending')")
            .expect("IndexedDB databases list state should evaluate")
            == "true"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("IndexedDB databases list should advance");
    }

    let result = vm
        .eval("String(globalThis.__indexedDbDatabasesResult)")
        .expect("indexedDB databases list result should be readable");

    assert_eq!(
        result,
        r#"{"promiseBrand":true,"entries":["alpha:1","beta:2"],"objectKeys":["name,version","name,version"],"firstNameDescriptor":{"value":"alpha","writable":true,"enumerable":true,"configurable":true},"firstVersionDescriptor":{"value":1,"writable":true,"enumerable":true,"configurable":true},"setterHits":[]}"#
    );
}

#[tokio::test]
async fn indexed_db_databases_resolves_from_child_message_handler() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://indexeddb-child-databases-message.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbChildDatabasesMessages = [];
  globalThis.__indexedDbChildDatabasesReady = false;
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <script>
      window.onmessage = () => {
        indexedDB.databases().then(
          () => parent.postMessage({ result: "resolved" }, "*"),
          error => parent.postMessage({ result: error && error.name }, "*")
        );
      };
      parent.postMessage({ kind: "ready" }, "*");
    </` + `script>`;
  window.onmessage = event => {
    if (event.data && event.data.kind === "ready") {
      globalThis.__indexedDbChildDatabasesReady = true;
      return;
    }
    globalThis.__indexedDbChildDatabasesMessages.push({
      data: event.data,
      sourceIsChild: event.source === frame.contentWindow
    });
  };
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__indexedDbChildDatabasesFrame = frame;
  return "ready";
})()
"#,
    )
    .expect("child databases message setup should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__indexedDbChildDatabasesReady)",
        "true",
        "srcdoc IndexedDB child should publish its ready fact",
    )
    .await;
    vm.eval("__indexedDbChildDatabasesFrame.contentWindow.postMessage({}, '*')")
        .expect("child databases message should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__indexedDbChildDatabasesMessages.length)",
        "1",
        "child IndexedDB databases response should arrive",
    )
    .await;

    let result = vm
        .eval("JSON.stringify(globalThis.__indexedDbChildDatabasesMessages)")
        .expect("child databases message result should evaluate");
    assert_eq!(
        result,
        r#"[{"data":{"result":"resolved"},"sourceIsChild":true}]"#
    );
}

#[test]
fn indexed_db_internal_slots_are_not_object_own_properties() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-private-slots.test/");

    vm.eval(
        r#"
(() => {
  const hiddenPrefixes = ["__moliIndexedDb", "moli.IndexedDb"];
  const leaks = {};
  function remember(name, value) {
    if (value === null || (typeof value !== "object" && typeof value !== "function")) {
      return value;
    }
    const own = Reflect.ownKeys(value)
      .map((key) => String(key))
      .filter((key) => hiddenPrefixes.some((prefix) => key.startsWith(prefix)));
    if (own.length) {
      leaks[name] = own;
    }
    return value;
  }

  globalThis.__indexedDbPrivateSlotVisibilityResult = "pending";
  const open = remember("openRequest", remember("factory", indexedDB).open("app", 1));
  open.onerror = () => {
    globalThis.__indexedDbPrivateSlotVisibilityResult = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    const db = remember("upgradeDatabase", open.result);
    const tx = remember("upgradeTransaction", open.transaction);
    const store = remember("upgradeObjectStore", db.createObjectStore("posts"));
    remember("upgradeIndex", store.createIndex("by-tag", "tag"));
    remember("upgradeTransactionAfterStore", tx);
  };
  open.onsuccess = () => {
    remember("openRequestDone", open);
    const db = remember("database", open.result);
    const writeTx = remember("writeTransaction", db.transaction("posts", "readwrite"));
    const writeStore = remember("writeObjectStore", writeTx.objectStore("posts"));
    const putReq = remember("putRequest", writeStore.put({ tag: "news", value: "one" }, "a"));
    putReq.onsuccess = () => remember("putRequestDone", putReq);
    writeTx.oncomplete = () => {
      remember("writeTransactionDone", writeTx);
      const readTx = remember("readTransaction", db.transaction("posts"));
      const readStore = remember("readObjectStore", readTx.objectStore("posts"));
      const index = remember("readIndex", readStore.index("by-tag"));
      const range = remember("keyRange", IDBKeyRange.only("news"));
      const cursorReq = remember("cursorRequest", index.openCursor(range));
      cursorReq.onsuccess = () => {
        remember("cursorRequestDone", cursorReq);
        const cursor = remember("cursor", cursorReq.result);
        if (!cursor) {
          globalThis.__indexedDbPrivateSlotVisibilityResult = "missing-cursor";
          return;
        }
        const leakKeys = Object.keys(leaks).sort();
        globalThis.__indexedDbPrivateSlotVisibilityResult = leakKeys.length
          ? `leaks:${JSON.stringify(leaks)}`
          : `ok:${cursor.key}:${cursor.primaryKey}:${cursor.value.value}`;
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb private slot visibility workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbPrivateSlotVisibilityResult)")
        .expect("indexeddb private slot visibility result should be readable");

    assert_eq!(result, "ok:news:a:one");
}

#[test]
fn indexed_db_request_declared_properties_preserve_own_enumerable_surface() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-request-enumerable.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbRequestEnumerableResult = "pending";
  const open = indexedDB.open(`app-${Math.random()}`, 1);
  const openKeys = Object.keys(open).sort();
  const openSourceIsNull = open.source === null;
  const openTransactionIsNull = open.transaction === null;
  const openHandlers = [
    Object.hasOwn(open, "onupgradeneeded"),
    Object.hasOwn(open, "onblocked")
  ];
  open.onupgradeneeded = () => open.result.createObjectStore("kv");
  open.onsuccess = () => {
    const db = open.result;
    const databaseKeys = Object.keys(db).sort();
    const databaseHandlers = [
      Object.hasOwn(db, "onabort"),
      Object.hasOwn(db, "onclose"),
      Object.hasOwn(db, "onerror"),
      Object.hasOwn(db, "onversionchange")
    ];
    const tx = db.transaction("kv");
    const transactionKeys = Object.keys(tx).sort();
    const transactionHandlers = [
      Object.hasOwn(tx, "onabort"),
      Object.hasOwn(tx, "oncomplete"),
      Object.hasOwn(tx, "onerror")
    ];
    const request = tx.objectStore("kv").get("missing");
    globalThis.__indexedDbRequestEnumerableResult = JSON.stringify({
      openKeys,
      openHandlers,
      databaseKeys,
      databaseHandlers,
      databaseVersion: db.version,
      databaseStores: db.objectStoreNames.contains("kv"),
      transactionKeys,
      transactionHandlers,
      transactionMode: tx.mode,
      transactionStores: tx.objectStoreNames.contains("kv"),
      requestKeys: Object.keys(request).sort(),
      requestHandlers: [
        Object.hasOwn(request, "onupgradeneeded"),
        Object.hasOwn(request, "onblocked")
      ],
      openSourceIsNull,
      openTransactionIsNull,
      initialReadyState: request.readyState
    });
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb request enumerable workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbRequestEnumerableResult)")
        .expect("indexeddb request enumerable result should be readable");

    assert_eq!(
        result,
        r#"{"openKeys":["error","onblocked","onerror","onsuccess","onupgradeneeded","readyState","result","source","transaction"],"openHandlers":[true,true],"databaseKeys":["name","objectStoreNames","onabort","onclose","onerror","onversionchange","version"],"databaseHandlers":[true,true,true,true],"databaseVersion":1,"databaseStores":true,"transactionKeys":["db","error","mode","objectStoreNames","onabort","oncomplete","onerror"],"transactionHandlers":[true,true,true],"transactionMode":"readonly","transactionStores":true,"requestKeys":["error","onerror","onsuccess","readyState","result","source","transaction"],"requestHandlers":[false,false],"openSourceIsNull":true,"openTransactionIsNull":true,"initialReadyState":"pending"}"#
    );
}

#[test]
fn indexed_db_dom_string_list_methods_are_declared() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-dom-string-list-declared.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbDomStringListResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
    const list = open.result.objectStoreNames;
    const prototype = Reflect.getPrototypeOf(list);
    const contains = Object.getOwnPropertyDescriptor(prototype, "contains");
    const item = Object.getOwnPropertyDescriptor(prototype, "item");
    const length = Object.getOwnPropertyDescriptor(prototype, "length");
    const index = Object.getOwnPropertyDescriptor(list, "0");
    const ArrayConstructor = Array;
    const originalValues = ArrayConstructor.prototype.values;
    const originalGetPrototypeOf = Object.getPrototypeOf;
    const poisoned = () => {
      throw new Error("public Array prototype was observed");
    };
    ArrayConstructor.prototype.values = poisoned;
    Object.getPrototypeOf = poisoned;
    globalThis.Array = undefined;
    try {
      list[0] = "changed";
      const deleted = delete list[0];
      let constructorError = "none";
      try {
        new DOMStringList();
      } catch (error) {
        constructorError = error && error.name;
      }
      globalThis.__indexedDbDomStringListResult = JSON.stringify({
        isArray: ArrayConstructor.isArray(list),
        brand: list instanceof DOMStringList,
        prototypeParent: Reflect.getPrototypeOf(prototype) === Object.prototype,
        ownNames: Object.getOwnPropertyNames(list).sort(),
        keys: Object.keys(list).sort(),
        indexDescriptor: [
          index.enumerable,
          index.writable,
          index.configurable,
          index.value
        ],
        mutationBlocked: list[0] === "kv" && deleted === false,
        prototypeDescriptors: [
          contains.enumerable,
          contains.writable,
          contains.configurable,
          contains.value.name,
          contains.value.length,
          item.enumerable,
          item.writable,
          item.configurable,
          item.value.name,
          item.value.length,
          length.enumerable,
          typeof length.get
        ],
        iteratorIntrinsic:
          prototype[Symbol.iterator] === originalValues &&
          [...list].join(",") === "kv",
        constructorError,
        containsKv: list.contains("kv"),
        containsMissing: list.contains("missing"),
        item0: list.item(0),
        item1Null: list.item(1) === null
      });
    } finally {
      ArrayConstructor.prototype.values = originalValues;
      Object.getPrototypeOf = originalGetPrototypeOf;
      globalThis.Array = ArrayConstructor;
    }
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb DOMStringList declaration workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbDomStringListResult)")
        .expect("indexeddb DOMStringList declaration result should be readable");

    assert_eq!(
        result,
        r#"{"isArray":false,"brand":true,"prototypeParent":true,"ownNames":["0"],"keys":["0"],"indexDescriptor":[true,false,true,"kv"],"mutationBlocked":true,"prototypeDescriptors":[true,true,true,"contains",1,true,true,true,"item",1,true,"function"],"iteratorIntrinsic":true,"constructorError":"TypeError","containsKv":true,"containsMissing":false,"item0":"kv","item1Null":true}"#
    );
}

#[test]
fn indexed_db_ignores_legacy_object_slot_names() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-legacy-object-slot-poison.test/");

    vm.eval(
        r#"
(() => {
  const legacySlots = [
    "__moliIndexedDbEventListeners",
    "__moliIndexedDbPendingResult",
    "__moliIndexedDbPendingError",
    "__moliIndexedDbRequestBlockedDispatched",
    "__moliIndexedDbDatabaseHandle",
    "__moliIndexedDbDatabaseMetadata",
    "__moliIndexedDbDatabaseUpgradeTransaction",
    "__moliIndexedDbDatabaseKey",
    "__moliIndexedDbDatabaseClosed",
    "__moliIndexedDbTransactionHandle",
    "__moliIndexedDbTransactionActive",
    "__moliIndexedDbTransactionFinished",
    "__moliIndexedDbTransactionAborted",
    "__moliIndexedDbTransactionStarted",
    "__moliIndexedDbTransactionStartScheduled",
    "__moliIndexedDbTransactionDbKey",
    "__moliIndexedDbTransactionOperationQueue",
    "__moliIndexedDbTransactionPending",
    "__moliIndexedDbTransactionCommitScheduled",
    "__moliIndexedDbTransactionAbortDispatched",
    "__moliIndexedDbObjectStoreName",
    "__moliIndexedDbObjectStoreMetadata",
    "__moliIndexedDbIndexMarker",
    "__moliIndexedDbKeyRangeMarker",
    "__moliIndexedDbCursorRequest",
    "__moliIndexedDbCursorEntries",
    "__moliIndexedDbCursorPosition",
    "__moliIndexedDbCursorKeyOnly",
    "__moliIndexedDbTaskKind"
  ];
  function poison(value) {
    if (value === null || (typeof value !== "object" && typeof value !== "function")) {
      return value;
    }
    for (const slot of legacySlots) {
      value[slot] = { poisoned: slot };
    }
    return value;
  }

  globalThis.__indexedDbLegacyObjectSlotResult = "pending";
  const open = poison(poison(indexedDB).open("app", 1));
  open.onerror = () => {
    globalThis.__indexedDbLegacyObjectSlotResult = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    const db = poison(open.result);
    poison(open.transaction);
    const store = poison(db.createObjectStore("posts"));
    poison(store.createIndex("by-tag", "tag"));
  };
  open.onsuccess = () => {
    poison(open);
    const db = poison(open.result);
    const tx = poison(db.transaction("posts", "readwrite"));
    const store = poison(tx.objectStore("posts"));
    const putReq = poison(store.put({ tag: "news", value: "one" }, "a"));
    putReq.onerror = () => {
      globalThis.__indexedDbLegacyObjectSlotResult = `put-error:${putReq.error && putReq.error.name}`;
    };
    tx.onerror = () => {
      globalThis.__indexedDbLegacyObjectSlotResult = `write-error:${tx.error && tx.error.name}`;
    };
    tx.oncomplete = () => {
      const readTx = poison(db.transaction("posts"));
      const readStore = poison(readTx.objectStore("posts"));
      const index = poison(readStore.index("by-tag"));
      const range = poison(IDBKeyRange.only("news"));
      const cursorReq = poison(index.openCursor(range));
      cursorReq.onerror = () => {
        globalThis.__indexedDbLegacyObjectSlotResult = `cursor-error:${cursorReq.error && cursorReq.error.name}`;
      };
      cursorReq.onsuccess = () => {
        poison(cursorReq);
        const cursor = poison(cursorReq.result);
        globalThis.__indexedDbLegacyObjectSlotResult = cursor
          ? `ok:${cursor.key}:${cursor.primaryKey}:${cursor.value.value}`
          : "missing-cursor";
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb legacy object slot poison workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbLegacyObjectSlotResult)")
        .expect("indexeddb legacy object slot poison result should be readable");

    assert_eq!(result, "ok:news:a:one");
}

#[test]
fn indexed_db_runtime_owner_ignores_object_prototype_pollution() {
    let mut vm = new_storage_page_task_executor_test_vm(
        "https://indexeddb-runtime-prototype-pollution.test/",
    );

    vm.eval(
        r#"
(() => {
  const poisonFactory = {
    open() {
      throw new Error("poisoned inherited factory");
    }
  };
  const pollutedKeys = [
    "factory",
    "factoryInitialized",
    "taskQueue",
    "openDatabases",
    "blockedOpenQueue",
    "readwriteTransactionQueue",
    "moli.IndexedDb.runtime.factory",
    "moli.IndexedDb.runtime.factoryInitialized",
    "moli.IndexedDb.runtime.taskQueue",
    "moli.IndexedDb.runtime.openDatabases",
    "moli.IndexedDb.runtime.blockedOpenQueue",
    "moli.IndexedDb.runtime.readwriteTransactionQueue"
  ];
  function cleanup() {
    for (const key of pollutedKeys) {
      delete Object.prototype[key];
    }
  }

  Object.prototype.factory = poisonFactory;
  Object.prototype.factoryInitialized = true;
  Object.prototype.taskQueue = [{ poisoned: "task" }];
  Object.prototype.openDatabases = [{ poisoned: "open" }];
  Object.prototype.blockedOpenQueue = [{ poisoned: "blocked" }];
  Object.prototype.readwriteTransactionQueue = [{ poisoned: "readwrite" }];
  Object.prototype["moli.IndexedDb.runtime.factory"] = poisonFactory;
  Object.prototype["moli.IndexedDb.runtime.factoryInitialized"] = true;
  Object.prototype["moli.IndexedDb.runtime.taskQueue"] = [{ poisoned: "private-task" }];
  Object.prototype["moli.IndexedDb.runtime.openDatabases"] = [{ poisoned: "private-open" }];
  Object.prototype["moli.IndexedDb.runtime.blockedOpenQueue"] = [{ poisoned: "private-blocked" }];
  Object.prototype["moli.IndexedDb.runtime.readwriteTransactionQueue"] = [{ poisoned: "private-readwrite" }];

  globalThis.__indexedDbRuntimePrototypePollutionResult = "pending";
  try {
    const factory = indexedDB;
    const openType = typeof factory.open;
    if (openType !== "function") {
      cleanup();
      globalThis.__indexedDbRuntimePrototypePollutionResult = `bad-factory:${openType}`;
      return "scheduled";
    }
    const open = factory.open("app", 1);
    open.onerror = () => {
      const name = open.error && open.error.name;
      cleanup();
      globalThis.__indexedDbRuntimePrototypePollutionResult = `open-error:${name}`;
    };
    open.onupgradeneeded = () => {
      open.result.createObjectStore("kv");
    };
    open.onsuccess = () => {
      const result = {
        ignoredFactory: factory !== poisonFactory,
        sameFactory: indexedDB === factory,
        brandedFactory: factory instanceof IDBFactory,
        brandedDatabase: open.result instanceof IDBDatabase,
        inheritedTaskQueueStillPoisoned: Object.prototype.taskQueue?.[0]?.poisoned === "task"
      };
      cleanup();
      globalThis.__indexedDbRuntimePrototypePollutionResult = JSON.stringify(result);
    };
  } catch (error) {
    cleanup();
    globalThis.__indexedDbRuntimePrototypePollutionResult = `throw:${error && error.message}`;
  }
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb runtime prototype pollution workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks(
            "String(globalThis.__indexedDbRuntimePrototypePollutionResult)",
        )
        .expect("indexeddb runtime prototype pollution result should be readable");

    assert_eq!(
        result,
        r#"{"ignoredFactory":true,"sameFactory":true,"brandedFactory":true,"brandedDatabase":true,"inheritedTaskQueueStillPoisoned":true}"#
    );
}

#[test]
fn indexed_db_dispatch_event_rejects_fake_receiver_without_host_panic() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-dispatch-fake-receiver.test/");

    let result = vm
        .eval(
            r#"
(() => {
  try {
    IDBRequest.prototype.dispatchEvent.call({}, new Event("success"));
    return "no-throw";
  } catch (error) {
    return `${error && error.name}:${error instanceof TypeError}`;
  }
})()
"#,
        )
        .expect("fake IDB dispatchEvent receiver probe should evaluate");

    assert_eq!(result, "TypeError:true");
}

#[test]
fn indexed_db_internal_dictionaries_ignore_object_prototype_pollution() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-internal-dict-pollution.test/");

    vm.eval(
        r#"
(() => {
  let inheritedSuccessCalled = false;
  const pollutedKeys = ["success", "posts", "by-tag", "indexes", "indexNames"];
  function cleanup() {
    for (const key of pollutedKeys) {
      delete Object.prototype[key];
    }
  }

  Object.prototype.success = [() => {
    inheritedSuccessCalled = true;
  }];
  Object.prototype.posts = { name: "poisoned-store", indexNames: [] };
  Object.prototype["by-tag"] = {
    name: "poisoned-index",
    keyPath: "wrong",
    unique: true,
    multiEntry: false
  };
  Object.prototype.indexes = { "by-tag": Object.prototype["by-tag"] };
  Object.prototype.indexNames = ["by-tag"];

  globalThis.__indexedDbInternalDictionaryPollutionResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onerror = () => {
    const name = open.error && open.error.name;
    cleanup();
    globalThis.__indexedDbInternalDictionaryPollutionResult = `open-error:${name}`;
  };
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("posts");
    store.createIndex("by-tag", "tag");
  };
  open.onsuccess = () => {
    const db = open.result;
    const writeTx = db.transaction("posts", "readwrite");
    const writeStore = writeTx.objectStore("posts");
    const putReq = writeStore.put({ tag: "news", value: "one" }, "a");
    putReq.onerror = () => {
      const name = putReq.error && putReq.error.name;
      cleanup();
      globalThis.__indexedDbInternalDictionaryPollutionResult = `put-error:${name}`;
    };
    writeTx.onerror = () => {
      const name = writeTx.error && writeTx.error.name;
      cleanup();
      globalThis.__indexedDbInternalDictionaryPollutionResult = `write-error:${name}`;
    };
    writeTx.oncomplete = () => {
      const readTx = db.transaction("posts");
      const readStore = readTx.objectStore("posts");
      const index = readStore.index("by-tag");
      const cursorReq = index.openCursor(IDBKeyRange.only("news"));
      cursorReq.onerror = () => {
        const name = cursorReq.error && cursorReq.error.name;
        cleanup();
        globalThis.__indexedDbInternalDictionaryPollutionResult = `cursor-error:${name}`;
      };
      cursorReq.onsuccess = () => {
        const cursor = cursorReq.result;
        const result = cursor
          ? {
              inheritedSuccessCalled,
              key: cursor.key,
              primaryKey: cursor.primaryKey,
              value: cursor.value.value
            }
          : { inheritedSuccessCalled, missingCursor: true };
        cleanup();
        globalThis.__indexedDbInternalDictionaryPollutionResult = JSON.stringify(result);
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb internal dictionary pollution workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks(
            "String(globalThis.__indexedDbInternalDictionaryPollutionResult)",
        )
        .expect("indexeddb internal dictionary pollution result should be readable");

    assert_eq!(
        result,
        r#"{"inheritedSuccessCalled":false,"key":"news","primaryKey":"a","value":"one"}"#
    );
}

#[test]
fn storage_runtime_containers_are_created_on_first_use() {
    let mut vm = new_storage_page_task_executor_test_vm("https://storage-lazy-runtime.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const slots = [
    "__moliWindowLocalStorage",
    "__moliWindowSessionStorage"
  ];
  const before = slots.map((name) => Object.prototype.hasOwnProperty.call(window, name));
  const localDescriptor = Object.getOwnPropertyDescriptor(window, "localStorage");
  const sessionDescriptor = Object.getOwnPropertyDescriptor(window, "sessionStorage");
  const local = localStorage;
  const afterLocal = slots.map((name) => Object.prototype.hasOwnProperty.call(window, name));
  const session = sessionStorage;
  const afterSession = slots.map((name) => Object.prototype.hasOwnProperty.call(window, name));
  local.setItem("k", "v");
  session.setItem("s", "w");
  return JSON.stringify({
    before,
    afterLocal,
    afterSession,
    descriptors: [
      !!localDescriptor,
      typeof localDescriptor?.get,
      !!sessionDescriptor,
      typeof sessionDescriptor?.get
    ],
    constructorType: typeof Storage,
    instances: [local instanceof Storage, session instanceof Storage],
    methods: [typeof local.getItem, typeof session.getItem],
    values: [local.getItem("k"), session.getItem("s")]
  });
})()
"#,
        )
        .expect("storage runtime state probe should evaluate");

    assert_eq!(
        result,
        r#"{"before":[false,false],"afterLocal":[false,false],"afterSession":[false,false],"descriptors":[true,"function",true,"function"],"constructorType":"function","instances":[true,true],"methods":["function","function"],"values":["v","w"]}"#
    );
}

#[test]
fn indexed_db_can_roundtrip_a_record() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-roundtrip.test/");

    vm.eval(
            r#"
(() => {
  globalThis.__indexedDbResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onerror = () => {
    globalThis.__indexedDbResult = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
    const db = open.result;
    const writeTx = db.transaction("kv", "readwrite");
    const writeStore = writeTx.objectStore("kv");
    const putReq = writeStore.put({ value: 42 }, "answer");
    putReq.onerror = () => {
      globalThis.__indexedDbResult = `put-error:${putReq.error && putReq.error.name}`;
    };
    writeTx.onerror = () => {
      globalThis.__indexedDbResult = `write-error:${writeTx.error && writeTx.error.name}`;
    };
    writeTx.oncomplete = () => {
      const readTx = db.transaction("kv");
      const store = readTx.objectStore("kv");
      const getReq = store.get("answer");
      getReq.onerror = () => {
        globalThis.__indexedDbResult = `get-error:${getReq.error && getReq.error.name}`;
      };
      getReq.onsuccess = () => {
        const keyReq = store.getKey("answer");
        keyReq.onerror = () => {
          globalThis.__indexedDbResult = `getkey-error:${keyReq.error && keyReq.error.name}`;
        };
        keyReq.onsuccess = () => {
          const keysReq = store.getAllKeys();
          keysReq.onerror = () => {
            globalThis.__indexedDbResult = `getallkeys-error:${keysReq.error && keysReq.error.name}`;
          };
          keysReq.onsuccess = () => {
            globalThis.__indexedDbResult = `${getReq.result.value}|${keyReq.result}|${keysReq.result.join(",")}|${db.objectStoreNames.contains("kv")}`;
          };
        };
      };
    };
  };
  return "scheduled";
})()
"#,
        )
        .expect("indexeddb workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbResult)")
        .expect("indexeddb result should be readable");

    assert_eq!(result, "42|answer|answer|true");
}

#[tokio::test]
async fn indexed_db_can_roundtrip_crypto_key_internal_slots() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://indexeddb-cryptokey.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbCryptoKeyResult = "pending";
  const dbName = `cryptokey-${Math.random()}`;
  const rejectName = promise => promise.then(
    () => "resolved",
    error => error.name
  );
  const open = indexedDB.open(dbName, 1);
  open.onerror = () => {
    globalThis.__indexedDbCryptoKeyResult = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
    const db = open.result;
    Promise.resolve().then(async () => {
      const hmacKey = await crypto.subtle.importKey(
        "raw",
        new Uint8Array([0x30, 0x11, 0x22, 0x33]),
        { name: "HMAC", hash: "SHA-256" },
        false,
        ["sign", "verify"]
      );
      hmacKey.extraProperty = "hi";
      const x25519PrivatePkcs8 = new Uint8Array([
        48, 46, 2, 1, 0, 48, 5, 6, 3, 43, 101, 110, 4, 34, 4, 32,
        200, 131, 142, 118, 208, 87, 223, 183, 216, 201, 90, 105,
        225, 56, 22, 10, 221, 99, 115, 253, 113, 164, 210, 118,
        187, 86, 227, 168, 27, 100, 255, 97
      ]);
      const x25519PublicSpki = new Uint8Array([
        48, 42, 48, 5, 6, 3, 43, 101, 110, 3, 33, 0, 28, 242,
        177, 230, 2, 46, 197, 55, 55, 30, 215, 245, 62, 84, 250,
        17, 84, 216, 62, 152, 235, 100, 234, 81, 250, 229, 179,
        48, 124, 254, 151, 6
      ]);
      const expectedX25519Bits = new Uint8Array([
        39, 104, 64, 157, 250, 185, 158, 194, 59, 140, 137, 185,
        63, 245, 136, 2, 149, 247, 97, 118, 8, 143, 137, 228,
        61, 254, 190, 126, 161, 149, 0, 8
      ]);
      const sameBytes = (left, right) => {
        const a = new Uint8Array(left);
        return a.length === right.length && a.every((value, index) => value === right[index]);
      };
      const x25519Private = await crypto.subtle.importKey(
        "pkcs8",
        x25519PrivatePkcs8,
        "X25519",
        false,
        ["deriveBits"]
      );
      const x25519Public = await crypto.subtle.importKey(
        "spki",
        x25519PublicSpki,
        "X25519",
        true,
        []
      );
      x25519Private.extraProperty = "hi";
      const writeTx = db.transaction("kv", "readwrite");
      writeTx.onerror = () => {
        globalThis.__indexedDbCryptoKeyResult = `write-error:${writeTx.error && writeTx.error.name}`;
      };
      const writeStore = writeTx.objectStore("kv");
      writeStore.put(hmacKey, "hmac");
      writeStore.put(x25519Private, "x25519-private");
      writeTx.oncomplete = () => {
        const readStore = db.transaction("kv").objectStore("kv");
        const hmacReq = readStore.get("hmac");
        hmacReq.onerror = () => {
          globalThis.__indexedDbCryptoKeyResult = `get-error:${hmacReq.error && hmacReq.error.name}`;
        };
        hmacReq.onsuccess = () => {
          const x25519Req = readStore.get("x25519-private");
          x25519Req.onerror = () => {
            globalThis.__indexedDbCryptoKeyResult = `get-error:${x25519Req.error && x25519Req.error.name}`;
          };
          x25519Req.onsuccess = () => {
            Promise.resolve().then(async () => {
              const hmacClone = hmacReq.result;
              const x25519Clone = x25519Req.result;
              const data = new TextEncoder().encode("indexeddb key clone");
              const signature = await crypto.subtle.sign("HMAC", hmacKey, data);
              const verified = await crypto.subtle.verify("HMAC", hmacClone, signature, data);
              const hmacExportName = await rejectName(crypto.subtle.exportKey("raw", hmacClone));
              const derived = await crypto.subtle.deriveBits(
                { name: "X25519", public: x25519Public },
                x25519Clone,
                256
              );
              const x25519ExportName = await rejectName(
                crypto.subtle.exportKey("pkcs8", x25519Clone)
              );
              globalThis.__indexedDbCryptoKeyResult = [
                String(hmacClone instanceof CryptoKey),
                String(hmacClone !== hmacKey),
                String(hmacClone.extraProperty),
                [hmacClone.type, hmacClone.extractable, hmacClone.algorithm.name, hmacClone.algorithm.hash.name, hmacClone.algorithm.length].join(":"),
                hmacClone.usages.join(","),
                String(verified),
                hmacExportName,
                String(x25519Clone instanceof CryptoKey),
                String(x25519Clone !== x25519Private),
                String(x25519Clone.extraProperty),
                [x25519Clone.type, x25519Clone.extractable, x25519Clone.algorithm.name].join(":"),
                x25519Clone.usages.join(","),
                String(sameBytes(derived, expectedX25519Bits)),
                x25519ExportName
              ].join("|");
            }).catch(error => {
              globalThis.__indexedDbCryptoKeyResult = `verify-error:${error.name}`;
            });
          };
        };
      };
    }).catch(error => {
      globalThis.__indexedDbCryptoKeyResult = `import-error:${error.name}`;
    });
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb CryptoKey workflow should schedule");

    for _ in 0..64 {
        while vm
            .run_one_webcrypto_task_executor_turn(&loader)
            .await
            .expect("WebCrypto production task should apply")
        {}
        if vm
            .eval("String(globalThis.__indexedDbCryptoKeyResult !== 'pending')")
            .expect("indexeddb CryptoKey state should evaluate")
            == "true"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("indexeddb CryptoKey workflow should advance");
        if vm
            .eval("String(globalThis.__indexedDbCryptoKeyResult !== 'pending')")
            .expect("indexeddb CryptoKey state should evaluate after one task")
            == "true"
        {
            break;
        }
        // Re-enter the loop before waiting: the task-end checkpoint above may
        // already have made WebCrypto runnable while its coalesced owner wake
        // was consumed by the selected Page turn. The production dispatcher
        // always checks stable sources before blocking; this fixture must do
        // the same instead of treating another wake as required progress.
    }

    let result = vm
        .eval("String(globalThis.__indexedDbCryptoKeyResult)")
        .expect("indexeddb CryptoKey result should be readable");

    assert_eq!(
        result,
        "true|true|undefined|secret:false:HMAC:SHA-256:32|sign,verify|true|InvalidAccessError|true|true|undefined|private:false:X25519|deriveBits|true|InvalidAccessError"
    );
}

#[test]
fn indexed_db_roundtrips_blob_file_and_array_buffer_values() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-blob-value.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbBlobResult = "pending";
  const dbName = `blob-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onerror = () => {
    globalThis.__indexedDbBlobResult = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    open.result.createObjectStore("values", { keyPath: "id" });
  };
  open.onsuccess = () => {
    const db = open.result;
    const blob = new Blob(["HELLO_IDB"], { type: "text/plain" });
    const file = new File(["FILE_IDB"], "note.txt", {
      type: "text/custom",
      lastModified: 123
    });
    const writeTx = db.transaction("values", "readwrite");
    writeTx.onerror = () => {
      globalThis.__indexedDbBlobResult = `write-error:${writeTx.error && writeTx.error.name}`;
    };
    writeTx.objectStore("values").put({
      id: 1,
      blob,
      file,
      buffer: new Uint8Array([1, 2, 3, 4, 5]).buffer
    });
    writeTx.oncomplete = () => {
      const get = db.transaction("values").objectStore("values").get(1);
      get.onerror = () => {
        globalThis.__indexedDbBlobResult = `get-error:${get.error && get.error.name}`;
      };
      get.onsuccess = () => {
        const row = get.result;
        Promise.all([row.blob.text(), row.file.text()]).then(([blobText, fileText]) => {
          const view = new Uint8Array(row.buffer);
          globalThis.__indexedDbBlobResult = [
            row.blob instanceof Blob,
            row.blob !== blob,
            row.blob.type,
            row.blob.size,
            blobText,
            row.file instanceof File,
            row.file instanceof Blob,
            row.file !== file,
            row.file.name,
            row.file.type,
            row.file.lastModified,
            fileText,
            view.byteLength,
            view[0]
          ].join("|");
        }, error => {
          globalThis.__indexedDbBlobResult = `blob-error:${error && error.name}`;
        });
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb Blob/File workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbBlobResult)")
        .expect("indexeddb Blob/File result should be readable");

    assert_eq!(
        result,
        "true|true|text/plain|9|HELLO_IDB|true|true|true|note.txt|text/custom|123|FILE_IDB|5|1"
    );
}

#[test]
fn indexed_db_roundtrips_opfs_handles_with_durable_external_objects() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-opfs-handle.test/");

    vm.eval(
        r#"
globalThis.__indexedDbOpfsHandleResult = "pending";
(async () => {
  const root = await navigator.storage.getDirectory();
  const directory = await root.getDirectoryHandle("durable-dir", { create: true });
  const file = await directory.getFileHandle("durable.txt", { create: true });
  const writer = await file.createWritable();
  await writer.write("durable bytes");
  await writer.close();

  const open = indexedDB.open(`opfs-${Math.random()}`, 1);
  open.onerror = () => {
    globalThis.__indexedDbOpfsHandleResult =
      `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => open.result.createObjectStore("values");
  open.onsuccess = () => {
    const db = open.result;
    const writeTx = db.transaction("values", "readwrite");
    const source = { root, directory, handles: [file, file], blob: new Blob(["blob bytes"]) };
    const put = writeTx.objectStore("values").put(source, "key");
    put.onerror = () => {
      globalThis.__indexedDbOpfsHandleResult =
        `put-error:${put.error && put.error.name}`;
    };
    writeTx.oncomplete = () => {
      const get = db.transaction("values").objectStore("values").get("key");
      get.onerror = () => {
        globalThis.__indexedDbOpfsHandleResult =
          `get-error:${get.error && get.error.name}`;
      };
      get.onsuccess = () => {
        Promise.resolve().then(async () => {
          const clone = get.result;
          const clonedFile = clone.handles[0];
          globalThis.__indexedDbOpfsHandleResult = JSON.stringify({
            rootBrand: clone.root instanceof FileSystemDirectoryHandle,
            directoryBrand: clone.directory instanceof FileSystemDirectoryHandle,
            fileBrand: clonedFile instanceof FileSystemFileHandle,
            distinctFromSource: clonedFile !== file,
            sharedReference: clone.handles[0] === clone.handles[1],
            sameEntry: await clonedFile.isSameEntry(file),
            resolved: await clone.root.resolve(clonedFile),
            text: await (await clonedFile.getFile()).text(),
            blobText: await clone.blob.text()
          });
        }).catch(error => {
          globalThis.__indexedDbOpfsHandleResult =
            `read-error:${error && error.name}:${error && error.message}`;
        });
      };
    };
  };
})().catch(error => {
  globalThis.__indexedDbOpfsHandleResult =
    `setup-error:${error && error.name}:${error && error.message}`;
});
"#,
    )
    .expect("IndexedDB OPFS handle workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbOpfsHandleResult)")
        .expect("IndexedDB OPFS handle result should be readable");
    assert_eq!(
        result,
        r#"{"rootBrand":true,"directoryBrand":true,"fileBrand":true,"distinctFromSource":true,"sharedReference":true,"sameEntry":true,"resolved":["durable-dir","durable.txt"],"text":"durable bytes","blobText":"blob bytes"}"#
    );
}

#[test]
fn indexed_db_durable_handle_does_not_rebind_after_named_bucket_recreate() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-opfs-bucket-recreate.test/");

    vm.eval(
        r#"
globalThis.__indexedDbOpfsBucketRecreateResult = "pending";
(async () => {
  const request = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const transaction = transaction => new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onabort = transaction.onerror = () => reject(transaction.error);
  });
  const open = indexedDB.open(`opfs-bucket-${Math.random()}`, 1);
  open.onupgradeneeded = () => open.result.createObjectStore("values");
  const db = await request(open);

  const oldBucket = await navigator.storageBuckets.open("durable-bucket");
  const oldRoot = await oldBucket.getDirectory();
  const oldFile = await oldRoot.getFileHandle("same-name.txt", { create: true });
  const oldWriter = await oldFile.createWritable();
  await oldWriter.write("old bytes");
  await oldWriter.close();

  const writeTx = db.transaction("values", "readwrite");
  const writeDone = transaction(writeTx);
  await request(writeTx.objectStore("values").put(oldFile, "old-handle"));
  await writeDone;

  await navigator.storageBuckets.delete("durable-bucket");
  const newBucket = await navigator.storageBuckets.open("durable-bucket");
  const newRoot = await newBucket.getDirectory();
  const newFile = await newRoot.getFileHandle("same-name.txt", { create: true });
  const newWriter = await newFile.createWritable();
  await newWriter.write("new bytes");
  await newWriter.close();

  const oldClone = await request(
    db.transaction("values").objectStore("values").get("old-handle")
  );
  const oldRead = await oldClone.getFile().then(
    file => `resolved:${file.size}`,
    error => `rejected:${error && error.name}`
  );
  globalThis.__indexedDbOpfsBucketRecreateResult = JSON.stringify({
    brand: oldClone instanceof FileSystemFileHandle,
    name: oldClone.name,
    distinctFromSource: oldClone !== oldFile,
    sameAsReplacement: await oldClone.isSameEntry(newFile),
    oldRead,
    newText: await (await newFile.getFile()).text()
  });
})().catch(error => {
  globalThis.__indexedDbOpfsBucketRecreateResult =
    `error:${error && error.name}:${error && error.message}`;
});
"#,
    )
    .expect("IndexedDB named-bucket durable handle workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbOpfsBucketRecreateResult)")
        .expect("IndexedDB named-bucket durable handle result should be readable");
    assert_eq!(
        result,
        r#"{"brand":true,"name":"same-name.txt","distinctFromSource":true,"sameAsReplacement":false,"oldRead":"rejected:NotFoundError","newText":"new bytes"}"#
    );
}

#[test]
fn indexed_db_opfs_handle_survives_profile_restart() {
    let page_url = "https://indexeddb-opfs-profile-restart.test/";
    let profile_root = indexed_db_test_root("opfs-handle-profile-restart");
    let bucket_metadata_path = profile_root.join("storage-buckets.json");
    let cache_storage_root = profile_root.join("cache-storage");
    let indexed_db_root = profile_root.join("indexed-db");
    let opfs_root = profile_root.join("opfs");
    std::fs::create_dir_all(&profile_root).expect("profile test root should be created");

    {
        let manager = crate::new_indexed_db_manager(Some(indexed_db_root.clone()))
            .expect("profile IndexedDB manager should initialize");
        let storage_service = moli_storage_service::StorageService::on_disk(&opfs_root)
            .expect("profile OPFS service should initialize");
        let bucket_store = crate::new_shared_json_storage_bucket_store_with_storage_service(
            &bucket_metadata_path,
            &cache_storage_root,
            &manager,
            storage_service,
        )
        .expect("profile bucket store should initialize");
        let mut vm = new_storage_page_task_executor_test_vm(page_url);
        vm.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)));
        vm.set_storage_bucket_store(bucket_store);

        vm.eval(
            r#"
globalThis.__indexedDbOpfsProfileWrite = "pending";
(async () => {
  const request = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const transaction = transaction => new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onabort = transaction.onerror = () => reject(transaction.error);
  });
  const bucket = await navigator.storageBuckets.open("profile-bucket");
  const root = await bucket.getDirectory();
  const file = await root.getFileHandle("profile.txt", { create: true });
  const writer = await file.createWritable();
  await writer.write("before restart");
  await writer.close();

  const open = indexedDB.open("durable-profile-db", 1);
  open.onupgradeneeded = () => open.result.createObjectStore("values");
  const db = await request(open);
  const writeTx = db.transaction("values", "readwrite");
  const writeDone = transaction(writeTx);
  await request(writeTx.objectStore("values").put(file, "handle"));
  await writeDone;
  db.close();
  globalThis.__indexedDbOpfsProfileWrite = "stored";
})().catch(error => {
  globalThis.__indexedDbOpfsProfileWrite =
    `error:${error && error.name}:${error && error.message}`;
});
"#,
        )
        .expect("profile OPFS handle write should schedule");
        assert_eq!(
            vm.eval_after_selected_page_tasks("String(globalThis.__indexedDbOpfsProfileWrite)")
                .expect("profile OPFS handle write should settle"),
            "stored"
        );
    }

    {
        let manager = crate::new_indexed_db_manager(Some(indexed_db_root.clone()))
            .expect("profile IndexedDB manager should reopen");
        let storage_service = moli_storage_service::StorageService::on_disk(&opfs_root)
            .expect("profile OPFS service should reopen");
        let bucket_store = crate::new_shared_json_storage_bucket_store_with_storage_service(
            &bucket_metadata_path,
            &cache_storage_root,
            &manager,
            storage_service,
        )
        .expect("profile bucket store should reopen");
        let mut vm = new_storage_page_task_executor_test_vm(page_url);
        vm.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)));
        vm.set_storage_bucket_store(bucket_store);

        vm.eval(
            r#"
globalThis.__indexedDbOpfsProfileRead = "pending";
(async () => {
  const request = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const db = await request(indexedDB.open("durable-profile-db"));
  const file = await request(
    db.transaction("values").objectStore("values").get("handle")
  );
  const bucket = await navigator.storageBuckets.open("profile-bucket");
  const root = await bucket.getDirectory();
  const before = await (await file.getFile()).text();
  const writer = await file.createWritable();
  await writer.write("after restart");
  await writer.close();
  const samePath = await root.getFileHandle("profile.txt");
  globalThis.__indexedDbOpfsProfileRead = JSON.stringify({
    brand: file instanceof FileSystemFileHandle,
    name: file.name,
    before,
    sameEntry: await file.isSameEntry(samePath),
    resolved: await root.resolve(file),
    after: await (await samePath.getFile()).text()
  });
  db.close();
})().catch(error => {
  globalThis.__indexedDbOpfsProfileRead =
    `error:${error && error.name}:${error && error.message}`;
});
"#,
        )
        .expect("profile OPFS handle read should schedule");
        assert_eq!(
            vm.eval_after_selected_page_tasks("String(globalThis.__indexedDbOpfsProfileRead)")
                .expect("profile OPFS handle read should settle"),
            r#"{"brand":true,"name":"profile.txt","before":"before restart","sameEntry":true,"resolved":["profile.txt"],"after":"after restart"}"#
        );
    }

    std::fs::remove_dir_all(&profile_root).expect("profile test root should be removed");
}

#[test]
fn indexed_db_uses_injected_storage_manager() {
    let page_url = "https://indexeddb-explicit-root.test/";
    let origin = "https://indexeddb-explicit-root.test";
    let root_a = indexed_db_test_root("explicit-a");
    let mut vm = new_storage_page_task_executor_test_vm(page_url);
    let manager_a = crate::new_indexed_db_manager(Some(root_a.clone()))
        .expect("page-local indexedDB manager should initialize");
    vm.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager_a)));

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbRootResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onerror = () => {
    globalThis.__indexedDbRootResult = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
    open.result.close();
    globalThis.__indexedDbRootResult = "ok";
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb explicit-root workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbRootResult)")
        .expect("indexeddb explicit-root result should be readable");

    assert_eq!(result, "ok");
    let storage_key = moli_storage_key::MoliStorageKey::first_party_from_url(
        &url::Url::parse(origin).expect("origin should parse"),
        None,
    )
    .serialized_storage_key();
    assert!(
        indexed_db_origin_file(&root_a, &storage_key).exists(),
        "page-local manager should receive the IndexedDB origin file"
    );

    let _ = std::fs::remove_dir_all(root_a);
}

#[tokio::test]
async fn indexed_db_databases_in_cross_origin_child_excludes_top_origin_names() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (child_url, child_server) = spawn_indexed_db_databases_child_server().await;
    let child_origin = Url::parse(&child_url)
        .expect("child url should parse")
        .origin()
        .ascii_serialization();
    let child_port = Url::parse(&child_url)
        .expect("child url should parse")
        .port()
        .expect("child url should include port");
    let top_port = if child_port == u16::MAX {
        child_port - 1
    } else {
        child_port + 1
    };
    let top_url = format!("http://[::1]:{top_port}/parent.html");
    let root = indexed_db_test_root("databases-cross-origin-child");
    let manager = crate::new_indexed_db_manager(Some(root.clone()))
        .expect("page-local indexedDB manager should initialize");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&top_url, &loader);
    vm.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)));

    let db_name = "top-origin-db";
    let db_name_literal = serde_json::to_string(db_name).expect("db name should serialize");
    let setup = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__topDatabaseReady = "pending";
  const request = indexedDB.open({db_name_literal}, 1);
  request.onupgradeneeded = () => {{
    request.result.createObjectStore("store");
  }};
  request.onerror = () => {{
    globalThis.__topDatabaseReady = `error:${{request.error && request.error.name}}`;
  }};
  request.onsuccess = () => {{
    request.result.close();
    globalThis.__topDatabaseReady = "ok";
  }};
  return "scheduled";
}})()
"#
        ))
        .expect("top origin database setup should evaluate");
    assert_eq!(setup, "scheduled");
    for _ in 0..8 {
        if vm
            .eval("String(globalThis.__topDatabaseReady)")
            .expect("top database state should evaluate")
            == "ok"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("top database open should advance");
    }
    assert_eq!(
        vm.eval("String(globalThis.__topDatabaseReady)")
            .expect("top database final state should evaluate"),
        "ok"
    );

    let child_url_literal = serde_json::to_string(&child_url).expect("child url should serialize");
    let setup = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__childDatabaseNames = "pending";
  globalThis.__indexedDbDatabasesChildLoaded = false;
  addEventListener("message", event => {{
    globalThis.__childDatabaseNames = JSON.stringify({{
      origin: event.origin,
      data: JSON.parse(String(event.data))
    }});
  }});
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  frame.onload = () => {{
    globalThis.__indexedDbDatabasesChildLoaded = true;
  }};
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__indexedDbDatabasesChildFrame = frame;
  return "queued";
}})()
"#
        ))
        .expect("cross-origin child setup should evaluate");
    assert_eq!(setup, "queued");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__indexedDbDatabasesChildLoaded)",
        "true",
        "cross-origin IndexedDB child should reach its load event",
    )
    .await;
    let request = child_server
        .await
        .expect("child server task should finish serving helper");
    assert!(
        request.starts_with("GET /indexeddb-databases-child.html "),
        "unexpected child request: {request:?}"
    );

    let child_origin_literal =
        serde_json::to_string(&child_origin).expect("child origin should serialize");
    vm.eval(&format!(
        "__indexedDbDatabasesChildFrame.contentWindow.postMessage({{action: 'delete', name: {db_name_literal}}}, {child_origin_literal})"
    ))
    .expect("delete message to cross-origin child should evaluate");
    for _ in 0..8 {
        if vm
            .eval("String(globalThis.__childDatabaseNames !== 'pending')")
            .expect("child delete response state should evaluate")
            == "true"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("child delete response should advance");
    }
    let result = vm
        .eval("String(globalThis.__childDatabaseNames)")
        .expect("child delete response should evaluate");
    assert_eq!(
        result,
        format!(r#"{{"origin":{child_origin_literal},"data":{{"ok":true,"deleted":true}}}}"#),
        "cross-origin child deleteDatabase continuation must report the child sender origin"
    );

    vm.eval(&format!(
        "globalThis.__childDatabaseNames = 'pending'; __indexedDbDatabasesChildFrame.contentWindow.postMessage({{action: 'get'}}, {child_origin_literal})"
    ))
    .expect("message to cross-origin child should evaluate");
    for _ in 0..8 {
        if vm
            .eval("String(globalThis.__childDatabaseNames !== 'pending')")
            .expect("child database response state should evaluate")
            == "true"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("child database response should advance");
    }

    let result = vm
        .eval("String(globalThis.__childDatabaseNames)")
        .expect("child database names should evaluate");
    assert_ne!(result, "pending", "child did not post database names");
    assert_eq!(
        result,
        format!(r#"{{"origin":{child_origin_literal},"data":{{"ok":true,"names":[]}}}}"#),
        "cross-origin child databases() must not include top-origin databases and must report the child sender origin"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn indexed_db_top_factory_ignores_stale_active_child_scope() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (child_url, child_server) = spawn_indexed_db_origin_isolation_child_server().await;
    let child_port = Url::parse(&child_url)
        .expect("child url should parse")
        .port()
        .expect("child url should include port");
    let top_port = if child_port == u16::MAX {
        child_port - 1
    } else {
        child_port + 1
    };
    let top_url = format!("http://[::1]:{top_port}/parent.html");
    let root = indexed_db_test_root("top-factory-stale-active-child");
    let manager = crate::new_indexed_db_manager(Some(root.clone()))
        .expect("page-local indexedDB manager should initialize");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&top_url, &loader);
    vm.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)));

    let child_url_literal = serde_json::to_string(&child_url).expect("child url should serialize");
    let setup = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__indexedDbOriginIsolationResult = "pending";
  globalThis.__indexedDbOriginIsolationChildLoaded = false;
  const openTopDatabase = name => new Promise((resolve, reject) => {{
    const deleteRequest = indexedDB.deleteDatabase(name);
    deleteRequest.onblocked = () => reject(new Error("blocked-delete"));
    deleteRequest.onerror = () => reject(deleteRequest.error || new Error("delete-error"));
    deleteRequest.onsuccess = () => {{
      const openRequest = indexedDB.open(name, 1);
      openRequest.onblocked = () => reject(new Error("blocked-open"));
      openRequest.onerror = () => reject(openRequest.error || new Error("open-error"));
      openRequest.onupgradeneeded = () => {{
        openRequest.result.createObjectStore("s");
      }};
      openRequest.onsuccess = () => {{
        openRequest.result.close();
        resolve("ok");
      }};
    }};
  }});
  addEventListener("message", async event => {{
    if (!event.data || event.data.kind !== "child-ready") {{
      return;
    }}
    try {{
      globalThis.__indexedDbOriginIsolationResult = await openTopDatabase("shared-origin-lock-db");
    }} catch (error) {{
      globalThis.__indexedDbOriginIsolationResult = `error:${{error && error.message}}`;
    }}
  }});
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  frame.onload = () => {{
    globalThis.__indexedDbOriginIsolationChildLoaded = true;
  }};
  (document.body || document.documentElement || document).appendChild(frame);
  return "queued";
}})()
"#
        ))
        .expect("origin isolation regression setup should evaluate");
    assert_eq!(setup, "queued");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__indexedDbOriginIsolationChildLoaded)",
        "true",
        "origin-isolation child should reach load through selected Page tasks",
    )
    .await;
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__indexedDbOriginIsolationResult !== 'pending')",
        "true",
        "origin isolation workflow should complete through selected Page tasks",
    )
    .await;
    let request = child_server
        .await
        .expect("child server task should finish serving helper");
    assert!(
        request.starts_with("GET /indexeddb-origin-isolation-child.html "),
        "unexpected child request: {request:?}"
    );

    assert_eq!(
        vm.eval("String(globalThis.__indexedDbOriginIsolationResult)")
            .expect("origin isolation final result should evaluate"),
        "ok",
        "top window IndexedDB operations must stay on the top origin even when child IDB tasks leave an active child scope pending"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn indexed_db_blocked_upgrade_result_database_keeps_opener_owner() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let root = indexed_db_test_root("blocked-upgrade-result-owner");
    let manager = crate::new_indexed_db_manager(Some(root.clone()))
        .expect("page-local indexedDB manager should initialize");
    let mut vm = new_broadcast_channel_page_test_vm_with_loader(
        "https://indexeddb-blocked-upgrade-owner.test/",
        &loader,
    );
    vm.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)));

    vm.eval(
        r#"
(() => {
  globalThis.__blockedUpgradeOwnerTopReady = "pending";
  globalThis.__blockedUpgradeOwnerTopVersionChange = "pending";
  globalThis.__blockedUpgradeOwnerTopVersionChangeBc = [];
  const topVersionChangeChannel = new BroadcastChannel("blocked-upgrade-top-versionchange-owner");
  topVersionChangeChannel.onmessage = event => {
    globalThis.__blockedUpgradeOwnerTopVersionChangeBc.push(`${event.data}:${event.origin}`);
  };
  const request = indexedDB.open("blocked-upgrade-owner-db", 1);
  request.onerror = () => {
    globalThis.__blockedUpgradeOwnerTopReady = `error:${request.error && request.error.name}`;
  };
  request.onupgradeneeded = () => {
    request.result.createObjectStore("s");
  };
  request.onsuccess = () => {
    globalThis.__blockedUpgradeOwnerTopDb = request.result;
    globalThis.__blockedUpgradeOwnerTopDb.onversionchange = () => {
      const sender = new BroadcastChannel("blocked-upgrade-top-versionchange-owner");
      sender.postMessage("top-versionchange");
      globalThis.__blockedUpgradeOwnerTopVersionChange = "closed";
      globalThis.__blockedUpgradeOwnerTopDb.close();
    };
    globalThis.__blockedUpgradeOwnerTopReady = "ok";
  };
  return "scheduled";
})()
"#,
    )
    .expect("blocked-upgrade owner top setup should schedule");
    for _ in 0..16 {
        if vm
            .eval("String(globalThis.__blockedUpgradeOwnerTopReady)")
            .expect("top blocked-upgrade owner state should evaluate")
            == "ok"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("top blocked-upgrade owner setup should advance");
    }
    assert_eq!(
        vm.eval("String(globalThis.__blockedUpgradeOwnerTopReady)")
            .expect("top blocked-upgrade owner final state should evaluate"),
        "ok"
    );

    vm.eval(
        r#"
(() => {
  globalThis.__blockedUpgradeOwnerMessages = [];
  addEventListener("message", event => {
    globalThis.__blockedUpgradeOwnerMessages.push({
      kind: event.data && event.data.kind,
      sourceIsChild: event.source === globalThis.__blockedUpgradeOwnerFrame.contentWindow
    });
  });
  const frame = document.createElement("iframe");
  globalThis.__blockedUpgradeOwnerFrame = frame;
  frame.srcdoc = `<!doctype html><meta charset="utf-8"><script>
let childDb = null;
addEventListener("message", event => {
  if (!event.data || event.data.action !== "open-v2")
    return;
  const request = indexedDB.open("blocked-upgrade-owner-db", 2);
  request.onblocked = () => {
    parent.postMessage({ kind: "child-open-v2-blocked" }, "*");
  };
  request.onerror = () => {
    parent.postMessage({ kind: "child-open-v2-error", error: request.error && request.error.name }, "*");
  };
  request.onupgradeneeded = () => {
    if (!request.result.objectStoreNames.contains("s"))
      request.result.createObjectStore("s");
  };
  request.onsuccess = () => {
    childDb = request.result;
    childDb.onversionchange = () => {
      parent.postMessage({ kind: "child-db-versionchange" }, "*");
      childDb.close();
    };
    parent.postMessage({ kind: "child-open-v2-success" }, "*");
  };
});
parent.postMessage({ kind: "child-ready" }, "*");
</` + `script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return "queued";
})()
"#,
    )
    .expect("blocked-upgrade owner child setup should schedule");

    for _ in 0..8 {
        if vm
            .eval(
                "String(globalThis.__blockedUpgradeOwnerMessages.some(message => message.kind === 'child-ready'))",
            )
            .expect("child-ready state should evaluate")
            == "true"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("blocked-upgrade owner child setup should advance");
    }

    vm.eval(
        r#"
globalThis.__blockedUpgradeOwnerFrame.contentWindow.postMessage({ action: "open-v2" }, "*");
"#,
    )
    .expect("child blocked-upgrade open should post");
    for _ in 0..24 {
        if vm
            .eval(
                "String(globalThis.__blockedUpgradeOwnerMessages.some(message => message.kind === 'child-open-v2-success'))",
            )
            .expect("child open-v2 state should evaluate")
            == "true"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("child open-v2 should advance");
    }
    assert_eq!(
        vm.eval("String(globalThis.__blockedUpgradeOwnerTopVersionChange)")
            .expect("top versionchange state should evaluate"),
        "closed",
        "child upgrade should have unblocked by closing the top connection"
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__blockedUpgradeOwnerTopVersionChangeBc.length > 0)",
        "true",
        "top versionchange BroadcastChannel delivery should advance",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__blockedUpgradeOwnerTopVersionChangeBc)",)
            .expect("top versionchange BroadcastChannel messages should evaluate"),
        r#"["top-versionchange:https://indexeddb-blocked-upgrade-owner.test"]"#,
        "top-owned versionchange handler must not inherit the child opener owner"
    );

    vm.eval(
        r#"
(() => {
  globalThis.__blockedUpgradeOwnerTopV3 = "pending";
  const request = indexedDB.open("blocked-upgrade-owner-db", 3);
  request.onblocked = () => {
    globalThis.__blockedUpgradeOwnerTopV3 = "blocked";
  };
  request.onerror = () => {
    globalThis.__blockedUpgradeOwnerTopV3 = `error:${request.error && request.error.name}`;
  };
  request.onupgradeneeded = () => {};
  request.onsuccess = () => {
    request.result.close();
    globalThis.__blockedUpgradeOwnerTopV3 = "ok";
  };
  return "scheduled";
})()
"#,
    )
    .expect("top v3 open should schedule");
    for _ in 0..24 {
        if vm
            .eval(
                "String(globalThis.__blockedUpgradeOwnerTopV3 !== 'pending' && globalThis.__blockedUpgradeOwnerMessages.some(message => message.kind === 'child-db-versionchange'))",
            )
            .expect("top v3 and child versionchange state should evaluate")
            == "true"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("top v3 open should advance");
    }

    assert_eq!(
        vm.eval("String(globalThis.__blockedUpgradeOwnerTopV3)")
            .expect("top v3 final state should evaluate"),
        "ok"
    );
    let result = vm
        .eval(
            r#"
JSON.stringify({
  childReady: globalThis.__blockedUpgradeOwnerMessages.some(
    message => message.kind === "child-ready" && message.sourceIsChild
  ),
  childOpen: globalThis.__blockedUpgradeOwnerMessages.some(
    message => message.kind === "child-open-v2-success" && message.sourceIsChild
  ),
  childVersionChange: globalThis.__blockedUpgradeOwnerMessages.some(
    message => message.kind === "child-db-versionchange" && message.sourceIsChild
  ),
  messages: globalThis.__blockedUpgradeOwnerMessages
})
"#,
        )
        .expect("blocked-upgrade owner messages should evaluate");
    assert_eq!(
        result,
        r#"{"childReady":true,"childOpen":true,"childVersionChange":true,"messages":[{"kind":"child-ready","sourceIsChild":true},{"kind":"child-open-v2-success","sourceIsChild":true},{"kind":"child-db-versionchange","sourceIsChild":true}]}"#,
        "database produced by a blocked child opener must keep the child owner for later database events"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn indexed_db_continuations_preserve_lightweight_popup_sender() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let root = indexed_db_test_root("databases-lightweight-popup");
    let manager = crate::new_indexed_db_manager(Some(root.clone()))
        .expect("page-local indexedDB manager should initialize");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://indexeddb-popup-databases-message.test/",
        &loader,
    );
    vm.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)));

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__popupIndexedDbMessages = [];
  onmessage = event => {
    globalThis.__popupIndexedDbMessages.push(JSON.stringify({
      origin: event.origin,
      sourceIsPopup: event.source === globalThis.__indexedDbPopup,
      data: JSON.parse(String(event.data))
    }));
  };
  const popup = open("about:blank#idb-popup");
  globalThis.__indexedDbPopup = popup;
  popup.onmessage = async event => {
    let response;
    try {
      if (event.data && event.data.action === "delete") {
        await new Promise((resolve, reject) => {
          const request = indexedDB.deleteDatabase(event.data.name);
          request.onsuccess = resolve;
          request.onerror = reject;
        });
        response = { ok: true, deleted: true };
      } else {
        const infos = await indexedDB.databases();
        response = { ok: true, names: infos.map(info => info.name) };
      }
    } catch (error) {
      response = { ok: false, error: error && error.name };
    }
    event.source.postMessage(JSON.stringify(response), event.origin);
  };
  popup.postMessage({ action: "delete", name: "popup-owner-db" }, "*");
  return "scheduled";
})()
"#,
        )
        .expect("popup IndexedDB owner workflow should schedule");
    assert_eq!(setup, "scheduled");

    for _ in 0..8 {
        if vm
            .eval("String(globalThis.__popupIndexedDbMessages.length)")
            .expect("popup delete response length should evaluate")
            == "1"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("wait driver should drain popup delete response");
    }
    assert_eq!(
        vm.eval("String(globalThis.__popupIndexedDbMessages[0])")
            .expect("popup delete response should evaluate"),
        r#"{"origin":"https://indexeddb-popup-databases-message.test","sourceIsPopup":true,"data":{"ok":true,"deleted":true}}"#
    );

    vm.eval(
        r#"
globalThis.__popupIndexedDbMessages = [];
globalThis.__indexedDbPopup.postMessage({ action: "get" }, "*");
"#,
    )
    .expect("popup databases message should evaluate");
    for _ in 0..8 {
        if vm
            .eval("String(globalThis.__popupIndexedDbMessages.length)")
            .expect("popup databases response length should evaluate")
            == "1"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("wait driver should drain popup databases response");
    }
    assert_eq!(
        vm.eval("String(globalThis.__popupIndexedDbMessages[0])")
            .expect("popup databases response should evaluate"),
        r#"{"origin":"https://indexeddb-popup-databases-message.test","sourceIsPopup":true,"data":{"ok":true,"names":[]}}"#
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn indexed_db_continuation_broadcast_channel_stays_in_lightweight_popup_owner() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let root = indexed_db_test_root("databases-popup-broadcast-channel-owner");
    let manager = crate::new_indexed_db_manager(Some(root.clone()))
        .expect("page-local indexedDB manager should initialize");
    let mut vm = new_broadcast_channel_page_test_vm_with_loader(
        "https://indexeddb-popup-broadcast-channel-owner.test/",
        &loader,
    );
    vm.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)));

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__popupIndexedDbBroadcastChannelMessages = [];
  const topChannel = new BroadcastChannel("indexeddb-popup-continuation-owner");
  topChannel.onmessage = event => {
    __popupIndexedDbBroadcastChannelMessages.push("top-bc:" + event.data + ":" + event.origin);
  };
  onmessage = event => {
    __popupIndexedDbBroadcastChannelMessages.push("window:" + event.data + ":" + event.origin);
  };

  const popup = open("https://indexeddb-popup-broadcast-channel-child.test/page.html");
  popup.onmessage = async event => {
    if (event.data !== "probe") {
      return;
    }
    try {
      await indexedDB.databases();
      const popupChannel = new BroadcastChannel("indexeddb-popup-continuation-owner");
      popupChannel.postMessage("from-popup-idb-continuation");
      event.source.postMessage("done", event.origin);
    } catch (error) {
      event.source.postMessage("error:" + (error && error.name), event.origin);
    }
  };
  popup.postMessage("probe", "*");
  return "scheduled";
})()
"#,
        )
        .expect("popup IndexedDB BroadcastChannel owner workflow should schedule");
    assert_eq!(setup, "scheduled");

    for _ in 0..16 {
        if vm
            .eval(
                r#"String(globalThis.__popupIndexedDbBroadcastChannelMessages.some(
  message => message.startsWith("window:")
))"#,
            )
            .expect("popup IndexedDB BroadcastChannel completion should evaluate")
            == "true"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("Page task executor should advance popup IndexedDB BroadcastChannel workflow");
    }

    vm.apply_pending_broadcast_channel_delivery_tasks(&loader, 4)
        .await
        .expect("any admitted BroadcastChannel executor tasks should apply");

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__popupIndexedDbBroadcastChannelMessages)")
            .expect("popup IndexedDB BroadcastChannel messages should evaluate"),
        r#"["window:done:https://indexeddb-popup-broadcast-channel-child.test"]"#
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn message_port_handler_indexed_db_uses_lightweight_popup_owner() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let root = indexed_db_test_root("message-port-popup-owner");
    let manager = crate::new_indexed_db_manager(Some(root.clone()))
        .expect("page-local indexedDB manager should initialize");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-port-popup-idb-owner.test/",
        &loader,
    );
    vm.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)));

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__messagePortPopupIndexedDbMessages = [];
  const openDb = name => new Promise((resolve, reject) => {
    const deleteRequest = indexedDB.deleteDatabase(name);
    deleteRequest.onblocked = () => reject(new Error("blocked-delete"));
    deleteRequest.onerror = () => reject(deleteRequest.error || new Error("delete-error"));
    deleteRequest.onsuccess = () => {
      const openRequest = indexedDB.open(name, 1);
      openRequest.onblocked = () => reject(new Error("blocked-open"));
      openRequest.onerror = () => reject(openRequest.error || new Error("open-error"));
      openRequest.onupgradeneeded = () => {
        openRequest.result.createObjectStore("s");
      };
      openRequest.onsuccess = () => {
        openRequest.result.close();
        resolve();
      };
    };
  });

  const popup = open("https://message-port-popup-idb-child.test/page.html");
  globalThis.__messagePortPopupIndexedDb = popup;
  onmessage = event => {
    globalThis.__messagePortPopupIndexedDbMessages.push(JSON.stringify({
      origin: event.origin,
      sourceIsPopup: event.source === popup,
      data: event.data
    }));
  };
  popup.onmessage = event => {
    if (event.data !== "setup") {
      return;
    }
    const channel = new MessageChannel();
    channel.port2.onmessage = async () => {
      const popupIndexedDB = indexedDB;
      let response;
      try {
        const databases = await popupIndexedDB.databases();
        response = {
          ok: true,
          names: databases.map(database => database.name).sort()
        };
      } catch (error) {
        response = {
          ok: false,
          error: error && error.name
        };
      }
      event.source.postMessage(response, event.origin);
    };
    channel.port1.postMessage("probe");
  };
  openDb("top-owner-db")
    .then(() => popup.postMessage("setup", "*"))
    .catch(error => {
      __messagePortPopupIndexedDbMessages.push("setup-error:" + (error && error.name));
    });
  return "scheduled";
})()
"#,
        )
        .expect("popup MessagePort IndexedDB owner workflow should schedule");
    assert_eq!(setup, "scheduled");

    for _ in 0..16 {
        if vm
            .eval("String(globalThis.__messagePortPopupIndexedDbMessages.length)")
            .expect("popup MessagePort IndexedDB response length should evaluate")
            == "1"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("popup MessagePort IndexedDB workflow should advance");
    }

    assert_eq!(
        vm.eval("String(globalThis.__messagePortPopupIndexedDbMessages[0])")
            .expect("popup MessagePort IndexedDB response should evaluate"),
        r#"{"origin":"https://message-port-popup-idb-child.test","sourceIsPopup":true,"data":{"ok":true,"names":[]}}"#
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn indexed_db_same_origin_popup_databases_share_top_level_owner() {
    let (popup_url, server) = spawn_indexed_db_databases_child_server().await;
    let popup_url = Url::parse(&popup_url).expect("popup URL should parse");
    let top_url = popup_url
        .join("/indexeddb-popup-parent.html")
        .expect("top URL should parse");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let root = indexed_db_test_root("same-origin-popup-databases");
    let manager = crate::new_indexed_db_manager(Some(root.clone()))
        .expect("page-local indexedDB manager should initialize");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(top_url.as_str(), &loader);
    vm.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)));

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbSameOriginDatabaseReady = "pending";
  const deleteRequest = indexedDB.deleteDatabase("shared-popup-owner-db");
  deleteRequest.onerror = () => {
    globalThis.__indexedDbSameOriginDatabaseReady = `delete-error:${deleteRequest.error && deleteRequest.error.name}`;
  };
  deleteRequest.onsuccess = () => {
    const openRequest = indexedDB.open("shared-popup-owner-db", 1);
    openRequest.onerror = () => {
      globalThis.__indexedDbSameOriginDatabaseReady = `open-error:${openRequest.error && openRequest.error.name}`;
    };
    openRequest.onupgradeneeded = () => {
      openRequest.result.createObjectStore("s");
    };
    openRequest.onsuccess = () => {
      openRequest.result.close();
      globalThis.__indexedDbSameOriginDatabaseReady = "ready";
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("same-origin popup IndexedDB setup should schedule");
    for _ in 0..16 {
        if vm
            .eval("String(globalThis.__indexedDbSameOriginDatabaseReady)")
            .expect("same-origin popup database setup state should evaluate")
            == "ready"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("same-origin popup database setup should advance");
    }
    assert_eq!(
        vm.eval("String(globalThis.__indexedDbSameOriginDatabaseReady)")
            .expect("same-origin popup database setup result should evaluate"),
        "ready"
    );

    let popup_url_literal =
        serde_json::to_string(popup_url.as_str()).expect("popup URL should serialize");
    let setup = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__indexedDbSameOriginPopupResult = "pending";
  let resolvePopupReady;
  const popupReady = new Promise(resolve => {{ resolvePopupReady = resolve; }});
  onmessage = event => {{
    if (event.data === "ready") {{
      resolvePopupReady();
      return;
    }}
    globalThis.__indexedDbSameOriginPopupResult = JSON.stringify({{
      origin: event.origin,
      sourceIsPopup: event.source === globalThis.__indexedDbSameOriginPopup,
      popupClosed: globalThis.__indexedDbSameOriginPopup.closed,
      data: JSON.parse(String(event.data))
    }});
  }};
  const popup = open({popup_url_literal}, "_blank");
  globalThis.__indexedDbSameOriginPopup = popup;
  popupReady.then(() => {{
    popup.postMessage({{ action: "get" }}, "*");
  }});
  return "scheduled";
}})()
"#
        ))
        .expect("same-origin popup IndexedDB workflow should schedule");
    assert_eq!(setup, "scheduled");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__indexedDbSameOriginPopupResult !== 'pending')",
        "true",
        "same-origin popup IndexedDB workflow should complete through selected Page tasks",
    )
    .await;
    let request = server
        .await
        .expect("popup server should finish serving helper");
    assert!(
        request.starts_with("GET /indexeddb-databases-child.html "),
        "unexpected popup request: {request:?}"
    );

    let expected_origin =
        serde_json::to_string(&origin_ascii_serialization(&top_url)).expect("origin should encode");
    assert_eq!(
        vm.eval("String(globalThis.__indexedDbSameOriginPopupResult)")
            .expect("same-origin popup databases result should evaluate"),
        format!(
            r#"{{"origin":{expected_origin},"sourceIsPopup":true,"popupClosed":true,"data":{{"ok":true,"names":["shared-popup-owner-db"]}}}}"#
        )
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn indexed_db_child_reply_does_not_leak_child_scope_to_top_continuation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let root = indexed_db_test_root("child-reply-top-continuation");
    let manager = crate::new_indexed_db_manager(Some(root.clone()))
        .expect("page-local indexedDB manager should initialize");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://indexeddb-child-reply-continuation.test/",
        &loader,
    );
    vm.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)));

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__childReplyScopeResult = "pending";
  function helperHtml(kind) {
    return `
      <!doctype html>
      <script>
        parent.postMessage({ kind: "ready", helper: ${JSON.stringify(kind)} }, "*");
        addEventListener("message", async event => {
          if (${JSON.stringify(kind)} === "first") {
            await indexedDB.databases();
            event.source.postMessage({ kind: "first-done" }, "*");
            return;
          }
          parent.postMessage({
            kind: "second-done",
            sourceIsNull: event.source === null,
            sourceIsTop: event.source === top
          }, "*");
        });
      <\/script>`;
  }
  async function appendHelper(kind) {
    const ready = new Promise(resolve => {
      const listener = event => {
        if (event.data && event.data.kind === "ready" && event.data.helper === kind) {
          removeEventListener("message", listener);
          resolve();
        }
      };
      addEventListener("message", listener);
    });
    const frame = document.createElement("iframe");
    frame.srcdoc = helperHtml(kind);
    (document.body || document.documentElement || document).appendChild(frame);
    await ready;
    return frame;
  }
  async function roundTrip(frame, payload) {
    const message = new Promise(resolve => {
      addEventListener("message", event => resolve(event.data), { once: true });
    });
    frame.contentWindow.postMessage(payload, "*");
    return await message;
  }
  (async () => {
    const first = await appendHelper("first");
    const firstReply = await roundTrip(first, { step: 1 });
    first.remove();

    const second = await appendHelper("second");
    const secondReply = await roundTrip(second, { step: 2 });
    __childReplyScopeResult = JSON.stringify({ firstReply, secondReply });
  })().catch(error => {
    __childReplyScopeResult = `error:${error && error.name}:${error && error.message}`;
  });
  return "scheduled";
})()
"#,
        )
        .expect("child reply continuation setup should evaluate");
    assert_eq!(setup, "scheduled");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__childReplyScopeResult !== 'pending')",
        "true",
        "child IndexedDB reply continuation",
    )
    .await;

    let result = vm
        .eval("String(globalThis.__childReplyScopeResult)")
        .expect("child reply continuation result should evaluate");
    assert_eq!(
        result,
        r#"{"firstReply":{"kind":"first-done"},"secondReply":{"kind":"second-done","sourceIsNull":false,"sourceIsTop":true}}"#
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn global_cache_storage_normalizes_request_info_urls() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://cache-request-info.test/app/index.html");

    vm.eval(
        r#"
globalThis.__cacheRequestInfoResult = "pending";
(async () => {
  await caches.delete("request-info");
  const cache = await caches.open("request-info");

  await cache.put(new Request("/root.txt"), new Response("request-key"));
  const requestPutStringMatch = await (await cache.match("/root.txt")).text();

  await cache.put("relative.txt", new Response("string-key"));
  const stringPutRequestMatch =
    await (await cache.match(new Request("relative.txt"))).text();

  await caches.delete("fragment-info");
  const fragmentCache = await caches.open("fragment-info");
  await fragmentCache.put(
    "/app/fragment.txt#put-fragment",
    new Response("fragment-key")
  );
  const fragmentKeys = await fragmentCache.keys();
  const fragmentMatch = await (
    await fragmentCache.match("/app/fragment.txt#match-fragment")
  ).text();
  const fragmentDeleted = await fragmentCache.delete(
    new Request("/app/fragment.txt#delete-fragment")
  );
  const fragmentDeleteMissing = await fragmentCache.delete(
    "/app/fragment.txt#missing-fragment"
  );

  return {
    requestPutStringMatch,
    stringPutRequestMatch,
    missing: typeof await cache.match("/missing.txt"),
    keysLength: fragmentCache.keys.length,
    deleteLength: fragmentCache.delete.length,
    fragmentKeyIsRequest: fragmentKeys[0] instanceof Request,
    fragmentKeys: fragmentKeys.map(request => request.url),
    fragmentMatch,
    fragmentDeleted,
    fragmentDeleteMissing,
    fragmentKeysAfterDelete: (await fragmentCache.keys()).length
  };
})().then(
  value => { globalThis.__cacheRequestInfoResult = JSON.stringify(value); },
  error => {
    globalThis.__cacheRequestInfoResult =
      `error:${error && error.name}:${error && error.message}`;
  }
);
"scheduled"
        "#,
    )
    .expect("global CacheStorage RequestInfo workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__cacheRequestInfoResult)")
        .expect("global CacheStorage RequestInfo result should be readable");

    assert_eq!(
        result,
        r#"{"requestPutStringMatch":"request-key","stringPutRequestMatch":"string-key","missing":"undefined","keysLength":0,"deleteLength":1,"fragmentKeyIsRequest":true,"fragmentKeys":["https://cache-request-info.test/app/fragment.txt#put-fragment"],"fragmentMatch":"fragment-key","fragmentDeleted":true,"fragmentDeleteMissing":false,"fragmentKeysAfterDelete":0}"#
    );
}

#[test]
fn global_cache_storage_applies_query_options_and_preserves_deleted_live_handles() {
    let mut vm = new_storage_page_task_executor_test_vm("https://cache-query.test/app/");

    vm.eval(
        r#"
globalThis.__cacheQueryResult = "pending";
(async () => {
  await caches.delete("query-options");
  const cache = await caches.open("query-options");
  const base = "/resource";
  await cache.put(`${base}?page=1`, new Response("page-one"));
  await cache.put(`${base}?page=2`, new Response("page-two"));
  await cache.put(
    new Request(`${base}?mode=vary`, { headers: { "X-Mode": "alpha" } }),
    new Response("vary-alpha", { headers: { Vary: "X-Mode" } })
  );
  const ignoredSearch = await (await cache.match(`${base}?page=99`, {
    ignoreSearch: true
  })).text();
  const varyMiss = typeof await cache.match(
    new Request(`${base}?mode=vary`, { headers: { "X-Mode": "beta" } })
  );
  const varyIgnored = await (await cache.match(
    new Request(`${base}?mode=vary`, { headers: { "X-Mode": "beta" } }),
    { ignoreVary: true }
  )).text();
  const methodMiss = typeof await cache.match(
    new Request(`${base}?page=2`, { method: "POST" })
  );
  const methodIgnored = await (await cache.match(
    new Request(`${base}?page=2`, { method: "POST" }),
    { ignoreMethod: true }
  )).text();
  const deletedExact = await cache.delete(`${base}?page=1`);
  const keys = (await cache.keys()).map(request => new URL(request.url).search.slice(1));
  const allBodies = await Promise.all((await cache.matchAll()).map(response => response.text()));
  const storageMatch = await (await caches.match(`${base}?page=2`)).text();

  await caches.delete("live-handle");
  const live = await caches.open("live-handle");
  await live.put("/old", new Response("old-value"));
  const deletedName = await caches.delete("live-handle");
  const hasAfterDelete = await caches.has("live-handle");
  const oldValue = await (await live.match("/old")).text();
  await live.put("/detached", new Response("detached-value"));
  const detachedValue = await (await live.match("/detached")).text();
  const reopened = await caches.open("live-handle");

  return {
    ignoredSearch,
    varyMiss,
    varyIgnored,
    methodMiss,
    methodIgnored,
    deletedExact,
    keys,
    allBodies,
    storageMatch,
    deletedName,
    hasAfterDelete,
    oldValue,
    detachedValue,
    reopenedKeys: (await reopened.keys()).length
  };
})().then(
  value => { globalThis.__cacheQueryResult = JSON.stringify(value); },
  error => {
    globalThis.__cacheQueryResult =
      `error:${error && error.name}:${error && error.message}`;
  }
);
"scheduled"
        "#,
    )
    .expect("global CacheStorage query workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__cacheQueryResult)")
        .expect("global CacheStorage query result should be readable");

    assert_eq!(
        result,
        r#"{"ignoredSearch":"page-one","varyMiss":"undefined","varyIgnored":"vary-alpha","methodMiss":"undefined","methodIgnored":"page-two","deletedExact":true,"keys":["page=2","mode=vary"],"allBodies":["page-two","vary-alpha"],"storageMatch":"page-two","deletedName":true,"hasAfterDelete":false,"oldValue":"old-value","detachedValue":"detached-value","reopenedKeys":0}"#
    );
}

#[test]
fn navigator_storage_estimate_usage_tracks_current_origin_storage() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    let page_url = format!("https://storage-estimate-{nonce}.test/");
    let mut vm = new_storage_page_task_executor_test_vm(&page_url);

    vm.eval(
        r#"
(() => {
  globalThis.__storageEstimateUsageResult = "pending";

  const openDb = () => new Promise((resolve, reject) => {
    const open = indexedDB.open("usage-db", 1);
    open.onerror = () => reject(open.error || new Error("open failed"));
    open.onupgradeneeded = () => {
      open.result.createObjectStore("kv");
    };
    open.onsuccess = () => resolve(open.result);
  });

  const waitForTransaction = (tx) => new Promise((resolve, reject) => {
    tx.oncomplete = resolve;
    tx.onabort = () => reject(tx.error || new Error("transaction aborted"));
    tx.onerror = () => reject(tx.error || new Error("transaction failed"));
  });

  navigator.storage.estimate()
    .then(async (before) => {
      localStorage.clear();
      sessionStorage.clear();
      localStorage.setItem("local", "abcd");
      sessionStorage.setItem("session", "xyz");
      const afterWebStorage = await navigator.storage.estimate();

      const db = await openDb();
      const afterEmptyIndexedDb = await navigator.storage.estimate();
      const tx = db.transaction("kv", "readwrite");
      tx.objectStore("kv").put("hello indexeddb", "record");
      await waitForTransaction(tx);
      const afterIndexedDb = await navigator.storage.estimate();
      db.close();

      const cache = await caches.open("estimate-cache");
      await cache.put("/resource", new Response("cache estimate bytes"));
      const afterCache = await navigator.storage.estimate();

      const root = await navigator.storage.getDirectory();
      const file = await root.getFileHandle("estimate.bin", { create: true });
      const writable = await file.createWritable();
      await writable.write("opfs estimate bytes");
      await writable.close();
      const afterOpfs = await navigator.storage.estimate();

      globalThis.__storageEstimateUsageResult = [
        before.usage,
        before.usageDetails && Object.keys(before.usageDetails).join(","),
        afterWebStorage.usage,
        afterWebStorage.usageDetails && Object.keys(afterWebStorage.usageDetails).join(","),
        afterEmptyIndexedDb.usage > afterWebStorage.usage,
        typeof afterEmptyIndexedDb.usageDetails.indexedDB,
        afterIndexedDb.usage > afterEmptyIndexedDb.usage,
        afterIndexedDb.usageDetails.indexedDB > (afterEmptyIndexedDb.usageDetails.indexedDB || 0),
        afterCache.usage > afterIndexedDb.usage,
        afterCache.usageDetails.caches > 0,
        afterOpfs.usage > afterCache.usage,
        afterOpfs.usageDetails.fileSystem > 0,
        Object.keys(afterOpfs.usageDetails).join(","),
        afterOpfs.quota
      ].join("|");
    })
    .catch((error) => {
      globalThis.__storageEstimateUsageResult = `error:${error && error.name}`;
    });

  return "scheduled";
})()
"#,
    )
    .expect("storage estimate usage workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__storageEstimateUsageResult)")
        .expect("storage estimate usage result should be readable");

    assert_eq!(
        result,
        "0||7||true|number|true|true|true|true|true|true|indexedDB,caches,fileSystem|1073741824"
    );

    let other_page_url = format!("https://storage-estimate-other-{nonce}.test/");
    let mut other_vm = new_storage_page_task_executor_test_vm(&other_page_url);
    other_vm
        .eval(
            r#"
globalThis.__storageEstimateOtherOriginResult = "pending";
navigator.storage.estimate().then((estimate) => {
  globalThis.__storageEstimateOtherOriginResult = String(estimate.usage);
});
"#,
        )
        .expect("other origin storage estimate should schedule");
    let other_result = other_vm
        .eval_after_selected_page_tasks("String(globalThis.__storageEstimateOtherOriginResult)")
        .expect("other origin storage estimate should be readable");
    assert_eq!(other_result, "0");
}

#[test]
fn default_bucket_quota_is_shared_by_cache_indexed_db_and_opfs() {
    let page_url = "https://default-aggregate-quota.test/";
    let mut vm = new_storage_page_task_executor_test_vm(page_url);
    let storage_key = moli_storage_key::MoliStorageKey::first_party_from_url(
        &Url::parse(page_url).unwrap(),
        None,
    )
    .serialized_storage_key();
    let reserved_for_other_backends = 1024 * 1024;
    let cache_usage =
        moli_storage_service::DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES - reserved_for_other_backends;
    {
        let mut store = vm.storage_bucket_store.lock();
        let identity = store
            .open_bucket(
                &storage_key,
                moli_storage_service::IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME,
            )
            .expect("implicit default CacheStorage metadata should open");
        assert_eq!(
            store
                .put_cache_entry_for_identity(
                    &identity,
                    "fixture",
                    "/reserved",
                    moli_storage_service::StorageBucketCachedResponse {
                        response_type: "default".to_owned(),
                        url: format!("{page_url}reserved"),
                        redirected: false,
                        status: 200,
                        status_text: "OK".to_owned(),
                        headers: Vec::new(),
                        body: b"small materialized fixture".to_vec(),
                    },
                    cache_usage,
                    0,
                )
                .expect("default cache usage fixture should store"),
            moli_storage_service::StorageBucketCachePutOutcome::Stored
        );
    }

    vm.eval(
        r#"
globalThis.__defaultAggregateQuotaResult = "pending";
(async () => {
  const outcome = async promise => {
    try {
      await promise;
      return "resolved";
    } catch (error) {
      return error && error.name;
    }
  };
  const request = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const root = await navigator.storage.getDirectory();
  const file = await root.getFileHandle("too-large.bin", { create: true });
  const writable = await file.createWritable();

  const open = indexedDB.open("aggregate-quota", 1);
  open.onupgradeneeded = () => open.result.createObjectStore("values");
  const db = await request(open);
  const cache = await caches.open("web");
  const before = await navigator.storage.estimate();

  const opfs = await outcome(writable.write(new Uint8Array(2 * 1024 * 1024)));
  const transaction = db.transaction("values", "readwrite");
  const indexedDbResult = await outcome(request(
    transaction.objectStore("values").put(new Uint8Array(2 * 1024 * 1024), "large")
  ));
  db.close();

  const cachesResult = await outcome(
    cache.put("/too-large", new Response(new Uint8Array(2 * 1024 * 1024)))
  );
  const after = await navigator.storage.estimate();
  return {
    quota: before.quota,
    cacheUsageReserved: before.usageDetails.caches > 1_000_000_000,
    opfs,
    indexedDB: indexedDbResult,
    caches: cachesResult,
    rejectedWritesDidNotGrowUsage: after.usage === before.usage,
    opfsUsageDidNotGrow:
      after.usageDetails.fileSystem === before.usageDetails.fileSystem
  };
})().then(
  value => { globalThis.__defaultAggregateQuotaResult = JSON.stringify(value); },
  error => {
    globalThis.__defaultAggregateQuotaResult =
      `error:${error && error.name}:${error && error.message}`;
  }
);
"scheduled"
        "#,
    )
    .expect("default aggregate quota probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__defaultAggregateQuotaResult)")
        .expect("default aggregate quota probe should settle");
    assert_eq!(
        result,
        r#"{"quota":1073741824,"cacheUsageReserved":true,"opfs":"QuotaExceededError","indexedDB":"QuotaExceededError","caches":"QuotaExceededError","rejectedWritesDidNotGrowUsage":true,"opfsUsageDidNotGrow":true}"#
    );
}

#[test]
fn indexed_db_transaction_mode_webidl_enum_surface() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-transaction-mode.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbTransactionModeResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onerror = () => {
    globalThis.__indexedDbTransactionModeResult = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
    const db = open.result;
    const values = [
      `default:${db.transaction("kv").mode}`,
      `readonly:${db.transaction("kv", "readonly").mode}`,
      `readwrite:${db.transaction("kv", "readwrite").mode}`,
    ];
    for (const mode of ["versionchange", "bogus"]) {
      try {
        db.transaction("kv", mode);
        values.push(`${mode}:accepted`);
      } catch (error) {
        values.push(`${mode}:${error.name}`);
      }
    }
    globalThis.__indexedDbTransactionModeResult = values.join("|");
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb transaction mode workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbTransactionModeResult)")
        .expect("indexeddb transaction mode result should be readable");

    assert_eq!(
        result,
        "default:readonly|readonly:readonly|readwrite:readwrite|versionchange:TypeError|bogus:TypeError"
    );
}

#[test]
fn indexed_db_onsuccess_throw_does_not_stop_success_listeners() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-success-handler-throw.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbDispatchOrder = [];
  globalThis.__indexedDbDispatchResult = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
    globalThis.__indexedDbDispatchOrder.push("onsuccess-before-throw");
    throw new Error("open onsuccess boom");
  };
  open.addEventListener("success", () => {
    globalThis.__indexedDbDispatchOrder.push("success-listener-after-on");
    globalThis.__indexedDbDispatchResult = String(
      open.result.objectStoreNames.contains("kv")
    );
  });
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb success handler throw workflow should schedule");

    let order = vm
        .eval_after_selected_page_tasks("JSON.stringify(globalThis.__indexedDbDispatchOrder)")
        .expect("indexeddb dispatch order should be readable");
    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbDispatchResult)")
        .expect("indexeddb dispatch result should be readable");

    assert_eq!(
        order,
        r#"["onsuccess-before-throw","success-listener-after-on"]"#
    );
    assert_eq!(result, "true");
}

#[test]
fn indexed_db_listener_added_during_handler_does_not_receive_current_event() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-event-snapshot.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbEventSnapshot = [];
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => {
    globalThis.__indexedDbEventSnapshot.push("handler");
    open.addEventListener("upgradeneeded", () => {
      globalThis.__indexedDbEventSnapshot.push("late-upgrade-listener");
    });
    open.addEventListener("success", () => {
      globalThis.__indexedDbEventSnapshot.push("success-listener");
    });
    open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
    globalThis.__indexedDbEventSnapshot.push("success-handler");
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb event snapshot workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("JSON.stringify(globalThis.__indexedDbEventSnapshot)")
        .expect("indexeddb event snapshot result should be readable");

    assert_eq!(
        result,
        r#"["handler","success-handler","success-listener"]"#
    );
}

#[test]
fn indexed_db_throwing_success_listener_does_not_stop_later_success_listeners() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-success-listener-throw.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbListenerOrder = [];
  globalThis.__indexedDbListenerResult = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
  };
  open.addEventListener("success", () => {
    globalThis.__indexedDbListenerOrder.push("listener-before-throw");
    throw new Error("open success listener boom");
  });
  open.addEventListener("success", () => {
    globalThis.__indexedDbListenerOrder.push("listener-after-throw");
    globalThis.__indexedDbListenerResult = String(
      open.result.objectStoreNames.contains("kv")
    );
  });
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb success listener throw workflow should schedule");

    let order = vm
        .eval_after_selected_page_tasks("JSON.stringify(globalThis.__indexedDbListenerOrder)")
        .expect("indexeddb listener order should be readable");
    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbListenerResult)")
        .expect("indexeddb listener result should be readable");

    assert_eq!(order, r#"["listener-before-throw","listener-after-throw"]"#);
    assert_eq!(result, "true");
}

#[test]
fn indexed_db_inline_auto_increment_injects_generated_key() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-inline-autoincrement.test/");

    vm.eval(
            r#"
(() => {
  globalThis.__indexedDbInlineAutoIncrementResult = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => {
    open.result.createObjectStore("posts", { keyPath: "id", autoIncrement: true });
  };
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("posts", "readwrite");
    const store = tx.objectStore("posts");
    const addReq = store.add({ title: "hello" });
    addReq.onerror = () => {
      globalThis.__indexedDbInlineAutoIncrementResult = `add-error:${addReq.error && addReq.error.name}`;
    };
    tx.oncomplete = () => {
      const readReq = db.transaction("posts").objectStore("posts").get(addReq.result);
      readReq.onerror = () => {
        globalThis.__indexedDbInlineAutoIncrementResult = `get-error:${readReq.error && readReq.error.name}`;
      };
      readReq.onsuccess = () => {
        globalThis.__indexedDbInlineAutoIncrementResult = [
          addReq.result,
          readReq.result.id,
          readReq.result.title
        ].join("|");
      };
    };
  };
  return "scheduled";
})()
"#,
        )
        .expect("indexeddb inline autoincrement workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbInlineAutoIncrementResult)")
        .expect("indexeddb inline autoincrement result should be readable");

    assert_eq!(result, "1|1|hello");
}

#[test]
fn indexed_db_nested_inline_auto_increment_injects_generated_key_path_suffix() {
    let mut vm = new_storage_page_task_executor_test_vm(
        "https://indexeddb-nested-inline-autoincrement.test/",
    );

    vm.eval(
            r#"
(() => {
  globalThis.__indexedDbNestedInlineAutoIncrementResult = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => {
    open.result.createObjectStore("posts", { keyPath: "meta.id", autoIncrement: true });
  };
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("posts", "readwrite");
    const store = tx.objectStore("posts");
    const addReq = store.add({ title: "hello" });
    addReq.onerror = () => {
      globalThis.__indexedDbNestedInlineAutoIncrementResult = `add-error:${addReq.error && addReq.error.name}`;
    };
    tx.oncomplete = () => {
      const readReq = db.transaction("posts").objectStore("posts").get(addReq.result);
      readReq.onerror = () => {
        globalThis.__indexedDbNestedInlineAutoIncrementResult = `get-error:${readReq.error && readReq.error.name}`;
      };
      readReq.onsuccess = () => {
        globalThis.__indexedDbNestedInlineAutoIncrementResult = [
          addReq.result,
          readReq.result.meta && readReq.result.meta.id,
          Object.prototype.hasOwnProperty.call(readReq.result, "meta"),
          Object.prototype.hasOwnProperty.call(readReq.result.meta || {}, "id"),
          readReq.result.title
        ].join("|");
      };
    };
  };
  return "scheduled";
})()
"#,
        )
        .expect("indexeddb nested inline autoincrement workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks(
            "String(globalThis.__indexedDbNestedInlineAutoIncrementResult)",
        )
        .expect("indexeddb nested inline autoincrement result should be readable");

    assert_eq!(result, "1|1|true|true|hello");
}

#[test]
fn indexed_db_put_function_throws_data_clone_error() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-dataclone.test/");

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__indexedDbCloneError = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => open.result.createObjectStore("kv");
  open.onsuccess = () => {
    try {
      open.result.transaction("kv", "readwrite").objectStore("kv").put(() => {}, "bad");
      globalThis.__indexedDbCloneError = "no-error";
    } catch (error) {
      globalThis.__indexedDbCloneError = error.name;
    }
  };
  return "scheduled";
})()
"#,
        )
        .expect("indexeddb dataclone workflow should schedule");

    assert_eq!(result, "scheduled");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbCloneError)")
        .expect("indexeddb dataclone result should be readable");

    assert_eq!(result, "DataCloneError");
}

#[test]
fn indexed_db_put_webassembly_module_throws_data_clone_error_for_storage() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-wasm-dataclone.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbWasmCloneError = "pending";
  const dbName = `app-${Math.random()}`;
  const createModule = () => new WebAssembly.Module(
    new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])
  );
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("store", { keyPath: "key" });

    let before = false;
    let after = false;
    const probe = (value) => {
      try {
        store.put(value);
        return "ok";
      } catch (error) {
        return error && error.name;
      }
    };

    globalThis.__indexedDbWasmCloneError = [
      probe({ key: 1, property: createModule() }),
      probe({
        key: 2,
        property: [
          { get x() { before = true; return 1; } },
          createModule(),
          { get x() { after = true; return 2; } }
        ]
      }),
      before,
      after
    ].join("|");
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb wasm dataclone workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbWasmCloneError)")
        .expect("indexeddb wasm dataclone result should be readable");

    assert_eq!(result, "DataCloneError|DataCloneError|true|false");
}

#[test]
fn indexed_db_inline_key_store_rejects_explicit_key_argument() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-inline-key-arg-error.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbInlineKeyArgError = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => open.result.createObjectStore("posts", { keyPath: "id" });
  open.onsuccess = () => {
    try {
      open.result.transaction("posts", "readwrite").objectStore("posts").put({ id: "a" }, "a");
      globalThis.__indexedDbInlineKeyArgError = "no-error";
    } catch (error) {
      globalThis.__indexedDbInlineKeyArgError = error.name;
    }
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb inline key argument workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbInlineKeyArgError)")
        .expect("indexeddb inline key argument result should be readable");

    assert_eq!(result, "DataError");
}

#[test]
fn indexed_db_inline_key_path_missing_key_throws_data_error() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-inline-missing-key-error.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbInlineMissingKeyError = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => open.result.createObjectStore("inline", { keyPath: "id" });
  open.onsuccess = () => {
    try {
      open.result.transaction("inline", "readwrite").objectStore("inline").add({ value: 1 });
    } catch (error) {
      globalThis.__indexedDbInlineMissingKeyError = error.name;
    }
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb inline missing key workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbInlineMissingKeyError)")
        .expect("indexeddb inline missing key result should be readable");

    assert_eq!(result, "DataError");
}

#[test]
fn indexed_db_out_of_line_store_missing_key_throws_data_error() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-outline-missing-key-error.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbOutOfLineMissingKeyError = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => open.result.createObjectStore("outline");
  open.onsuccess = () => {
    try {
      open.result.transaction("outline", "readwrite").objectStore("outline").add({ value: 1 });
    } catch (error) {
      globalThis.__indexedDbOutOfLineMissingKeyError = error.name;
    }
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb out-of-line missing key workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbOutOfLineMissingKeyError)")
        .expect("indexeddb out-of-line missing key result should be readable");

    assert_eq!(result, "DataError");
}

#[test]
fn indexed_db_rejects_out_of_range_integer_keys() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-out-of-range-key.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbOutOfRangeKeyError = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => open.result.createObjectStore("outline");
  open.onsuccess = () => {
    try {
      open.result
        .transaction("outline", "readwrite")
        .objectStore("outline")
        .put({ value: 1 }, Number.MAX_SAFE_INTEGER + 1);
      globalThis.__indexedDbOutOfRangeKeyError = "no-error";
    } catch (error) {
      globalThis.__indexedDbOutOfRangeKeyError = error.name;
    }
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb out-of-range key workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbOutOfRangeKeyError)")
        .expect("indexeddb out-of-range key result should be readable");

    assert_eq!(result, "TypeError");
}

#[test]
fn indexed_db_explicit_integer_key_advances_auto_increment_generator() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-autoincrement-generator.test/");

    vm.eval(
            r#"
(() => {
  globalThis.__indexedDbAutoIncrementGeneratorResult = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => open.result.createObjectStore("kv", { autoIncrement: true });
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("kv", "readwrite");
    const store = tx.objectStore("kv");
    const putReq = store.put("five", 5);
    putReq.onerror = () => {
      globalThis.__indexedDbAutoIncrementGeneratorResult = `put-error:${putReq.error && putReq.error.name}`;
    };
    putReq.onsuccess = () => {
      const addReq = store.add("six");
      addReq.onerror = () => {
        globalThis.__indexedDbAutoIncrementGeneratorResult = `add-error:${addReq.error && addReq.error.name}`;
      };
      addReq.onsuccess = () => {
        tx.oncomplete = () => {
          globalThis.__indexedDbAutoIncrementGeneratorResult = `${putReq.result}|${addReq.result}`;
        };
      };
    };
  };
  return "scheduled";
})()
"#,
        )
        .expect("indexeddb auto increment generator workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks(
            "String(globalThis.__indexedDbAutoIncrementGeneratorResult)",
        )
        .expect("indexeddb auto increment generator result should be readable");

    assert_eq!(result, "5|6");
}

#[test]
fn indexed_db_object_store_index_metadata_is_available() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-index-metadata.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbIndexResult = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onerror = () => {
    globalThis.__indexedDbIndexResult = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("kv", { keyPath: "id" });
    const created = store.createIndex("by-id", "id", { unique: true });
    const viaLookup = store.index("by-id");
    store.deleteIndex("by-id");
    store.createIndex("by-id", "id", { unique: true });
    globalThis.__indexedDbIndexResult = [
      Object.keys(store).sort().join(","),
      Object.hasOwn(store, "db"),
      Object.hasOwn(store, "transaction"),
      store.autoIncrement,
      Object.keys(created).sort().join(","),
      Object.hasOwn(created, "objectStore"),
      store.indexNames.contains("by-id"),
      created.name,
      created.keyPath,
      created.unique,
      created.multiEntry,
      viaLookup.name,
      viaLookup.objectStore.name
    ].join("|");
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb index workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbIndexResult)")
        .expect("indexeddb index metadata result should be readable");

    assert_eq!(
        result,
        "autoIncrement,db,indexNames,keyPath,name,transaction|true|true|false|keyPath,multiEntry,name,objectStore,unique|true|true|by-id|id|true|false|by-id|kv"
    );
}

#[test]
fn indexed_db_unique_index_rejects_duplicate_values() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-unique-index.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbUniqueConstraintResult = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("posts", { keyPath: "id" });
    store.createIndex("by-slug", "slug", { unique: true });
  };
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("posts", "readwrite");
    const store = tx.objectStore("posts");
    store.put({ id: "a", slug: "dup" });
    const second = store.put({ id: "b", slug: "dup" });
    second.onerror = () => {
      globalThis.__indexedDbUniqueConstraintResult = second.error && second.error.name;
    };
    tx.oncomplete = () => {
      if (globalThis.__indexedDbUniqueConstraintResult === "pending") {
        globalThis.__indexedDbUniqueConstraintResult = "complete-without-error";
      }
    };
    tx.onerror = () => {
      if (globalThis.__indexedDbUniqueConstraintResult === "pending") {
        globalThis.__indexedDbUniqueConstraintResult = `tx-error:${tx.error && tx.error.name}`;
      }
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("unique index workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbUniqueConstraintResult)")
        .expect("unique index result should be readable");

    assert_eq!(result, "ConstraintError");
}

#[test]
fn indexed_db_index_queries_and_key_ranges_work() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-index-query.test/");

    vm.eval(
            r#"
(() => {
  globalThis.__indexedDbIndexQueryResult = "pending";
    const open = indexedDB.open("app", 1);
    open.onerror = () => {
      globalThis.__indexedDbIndexQueryResult = `open-error:${open.error && open.error.name}`;
    };
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("posts", { keyPath: "id" });
    store.createIndex("by-tag", "tag");
  };
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("posts", "readwrite");
    const store = tx.objectStore("posts");
    store.put({ id: "a", tag: "news", score: 1 });
    store.put({ id: "b", tag: "tech", score: 2 });
    store.put({ id: "c", tag: "news", score: 3 });
    tx.oncomplete = () => {
      try {
        const readTx = db.transaction("posts");
        readTx.onerror = () => {
          globalThis.__indexedDbIndexQueryResult = `readtx-error:${readTx.error && readTx.error.name}`;
        };
        const index = readTx.objectStore("posts").index("by-tag");
        const range = IDBKeyRange.only("news");
        const allReq = index.getAll(range);
        allReq.onerror = () => {
          globalThis.__indexedDbIndexQueryResult = `getall-error:${allReq.error && allReq.error.name}`;
        };
        allReq.onsuccess = () => {
          const getReq = index.get("news");
          getReq.onerror = () => {
            globalThis.__indexedDbIndexQueryResult = `get-error:${getReq.error && getReq.error.name}`;
          };
          getReq.onsuccess = () => {
            const keyReq = index.getKey(range);
            keyReq.onerror = () => {
              globalThis.__indexedDbIndexQueryResult = `getkey-error:${keyReq.error && keyReq.error.name}`;
            };
            keyReq.onsuccess = () => {
              const keysReq = index.getAllKeys(range);
              keysReq.onerror = () => {
                globalThis.__indexedDbIndexQueryResult = `getallkeys-error:${keysReq.error && keysReq.error.name}`;
              };
              keysReq.onsuccess = () => {
                const countReq = index.count(range);
                countReq.onerror = () => {
                  globalThis.__indexedDbIndexQueryResult = `count-error:${countReq.error && countReq.error.name}`;
                };
                countReq.onsuccess = () => {
                  globalThis.__indexedDbIndexQueryResult = [
                    Object.keys(range).sort().join(","),
                    range.includes("news"),
                    range.lower,
                    range.upper,
                    range.lowerOpen,
                    range.upperOpen,
                    getReq.result.id,
                    keyReq.result,
                    allReq.result.map((item) => item.id).join(","),
                    keysReq.result.join(","),
                    countReq.result
                  ].join("|");
                };
              };
            };
          };
        };
      } catch (error) {
        globalThis.__indexedDbIndexQueryResult = `sync-error:${error && error.name}`;
      }
    };
  };
  return "scheduled";
})()
"#,
        )
        .expect("indexeddb index query workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbIndexQueryResult)")
        .expect("indexeddb index query result should be readable");

    assert_eq!(
        result,
        "lower,lowerOpen,upper,upperOpen|true|news|news|false|false|a|a|a,c|a,c|2"
    );
}

#[test]
fn indexed_db_object_store_key_ranges_work() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-store-range.test/");

    vm.eval(
            r#"
(() => {
  globalThis.__indexedDbStoreRangeResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onerror = () => {
    globalThis.__indexedDbStoreRangeResult = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
    const db = open.result;
    const writeTx = db.transaction("kv", "readwrite");
    const store = writeTx.objectStore("kv");
    store.put("one", "a");
    store.put("two", "b");
    store.put("three", "c");
    writeTx.oncomplete = () => {
      const range = IDBKeyRange.bound("a", "b");
      const readTx = db.transaction("kv");
      const readStore = readTx.objectStore("kv");
      const getReq = readStore.get(range);
      getReq.onerror = () => {
        globalThis.__indexedDbStoreRangeResult = `get-error:${getReq.error && getReq.error.name}`;
      };
      getReq.onsuccess = () => {
        const keyReq = readStore.getKey(range);
        keyReq.onerror = () => {
          globalThis.__indexedDbStoreRangeResult = `getkey-error:${keyReq.error && keyReq.error.name}`;
        };
        keyReq.onsuccess = () => {
          const allReq = readStore.getAll(IDBKeyRange.lowerBound("b"), 2);
          allReq.onerror = () => {
            globalThis.__indexedDbStoreRangeResult = `getall-error:${allReq.error && allReq.error.name}`;
          };
          allReq.onsuccess = () => {
            const keysReq = readStore.getAllKeys(range, 2);
            keysReq.onerror = () => {
              globalThis.__indexedDbStoreRangeResult = `getallkeys-error:${keysReq.error && keysReq.error.name}`;
            };
            keysReq.onsuccess = () => {
              const countReq = readStore.count(IDBKeyRange.upperBound("b"));
              countReq.onerror = () => {
                globalThis.__indexedDbStoreRangeResult = `count-error:${countReq.error && countReq.error.name}`;
              };
              countReq.onsuccess = () => {
                globalThis.__indexedDbStoreRangeResult = [
                  getReq.result,
                  keyReq.result,
                  allReq.result.join(","),
                  keysReq.result.join(","),
                  countReq.result
                ].join("|");
              };
            };
          };
        };
      };
    };
  };
  return "scheduled";
})()
"#,
        )
        .expect("indexeddb object store range workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbStoreRangeResult)")
        .expect("indexeddb object store range result should be readable");

    assert_eq!(result, "one|a|two,three|a,b|2");
}

#[test]
fn indexed_db_object_store_range_count_zero_and_open_bounds_work() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-store-range-open.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbStoreRangeOpenResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => open.result.createObjectStore("kv");
  open.onsuccess = () => {
    const db = open.result;
    const writeTx = db.transaction("kv", "readwrite");
    const store = writeTx.objectStore("kv");
    store.put("one", "a");
    store.put("two", "b");
    store.put("three", "c");
    writeTx.oncomplete = () => {
      const tx = db.transaction("kv");
      const store = tx.objectStore("kv");
      const range = IDBKeyRange.bound("a", "c", true, true);
      const getAll = store.getAll(range, 0);
      getAll.onsuccess = () => {
        const keys = store.getAllKeys(range);
        keys.onsuccess = () => {
          const count = store.count(range);
          count.onsuccess = () => {
            globalThis.__indexedDbStoreRangeOpenResult = [
              range.includes("a"),
              range.includes("b"),
              range.includes("c"),
              getAll.result.length,
              keys.result.join(","),
              count.result
            ].join("|");
          };
        };
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb object store open range workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbStoreRangeOpenResult)")
        .expect("indexeddb object store open range result should be readable");

    assert_eq!(result, "false|true|false|0|b|1");
}

#[test]
fn indexed_db_get_all_object_wrapped_query_values_stay_queries() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-getall-wrapper-query.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbGetAllWrapperQueryResult = "pending";
  const open = indexedDB.open(`app-${Math.random()}`, 1);
  open.onupgradeneeded = () => open.result.createObjectStore("kv");
  open.onsuccess = () => {
    const db = open.result;
    const writeTx = db.transaction("kv", "readwrite");
    const store = writeTx.objectStore("kv");
    store.put("five", 5);
    store.put("string", "x");
    store.put("date", 1000);
    writeTx.oncomplete = () => {
      const readStore = db.transaction("kv").objectStore("kv");
      const numberReq = readStore.getAll(new Number(5));
      numberReq.onsuccess = () => {
        const stringReq = readStore.getAllKeys(new String("x"));
        stringReq.onsuccess = () => {
          const dateReq = readStore.getAll(new Date(1000));
          dateReq.onsuccess = () => {
            globalThis.__indexedDbGetAllWrapperQueryResult = [
              numberReq.result.join(","),
              stringReq.result.join(","),
              dateReq.result.join(",")
            ].join("|");
          };
        };
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb wrapper query workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbGetAllWrapperQueryResult)")
        .expect("indexeddb wrapper query result should be readable");

    assert_eq!(result, "five|x|date");
}

#[tokio::test]
async fn indexed_db_get_all_options_rejects_detached_typed_array_query() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://indexeddb-getall-detached-query.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbDetachedQueryResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => open.result.createObjectStore("kv");
  open.onsuccess = () => {
    const store = open.result.transaction("kv").objectStore("kv");
    const array = new Uint8Array([1, 2, 3, 4]);
    const channel = new MessageChannel();
    channel.port1.postMessage("", [array.buffer]);
    let dictionaryError = "none";
    let directError = "none";
    try {
      store.getAll({ query: array });
    } catch (error) {
      dictionaryError = error && error.name;
    }
    try {
      store.getAll(array);
    } catch (error) {
      directError = error && error.name;
    }
    globalThis.__indexedDbDetachedQueryResult = `${dictionaryError}:${directError}:${array.byteLength}`;
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb detached query workflow should schedule");

    for _ in 0..8 {
        if vm
            .eval("String(globalThis.__indexedDbDetachedQueryResult !== 'pending')")
            .expect("indexeddb detached query state should evaluate")
            == "true"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("indexeddb detached query should advance");
    }

    let result = vm
        .eval("String(globalThis.__indexedDbDetachedQueryResult)")
        .expect("indexeddb detached query result should be readable");

    assert_eq!(result, "DataError:DataError:0");
}

#[tokio::test]
async fn indexed_db_detached_realm_methods_keep_receiver_realm_state() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://indexeddb-cross-realm-method.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbCrossRealmMethodResult = "pending";

  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  const detachedStoreGet = frame.contentWindow.IDBObjectStore.prototype.get;
  const detachedIndexOpenCursor = frame.contentWindow.IDBIndex.prototype.openCursor;
  frame.remove();

  const open = indexedDB.open("app", 1);
  open.onerror = () => {
    globalThis.__indexedDbCrossRealmMethodResult = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("records");
    store.createIndex("by-kind", "kind");
    store.put({ kind: "entry", value: 7 }, 1);
  };
  open.onsuccess = () => {
    const transaction = open.result.transaction("records");
    const store = transaction.objectStore("records");
    const index = store.index("by-kind");
    const getRequest = detachedStoreGet.call(store, 1);
    const cursorRequest = detachedIndexOpenCursor.call(index);
    const observed = {};

    const finish = () => {
      if (!("get" in observed) || !("cursor" in observed))
        return;
      globalThis.__indexedDbCrossRealmMethodResult = JSON.stringify({
        getRequestIsMain: getRequest instanceof IDBRequest,
        getSourceIsStore: getRequest.source === store,
        getTransactionIsTransaction: getRequest.transaction === transaction,
        getResultUsesMainObjectPrototype:
          Object.getPrototypeOf(observed.get) === Object.prototype,
        getResultPrototypeIsNull: Object.getPrototypeOf(observed.get) === null,
        getResultConstructorIsMain: observed.get.constructor === Object,
        getResultConstructorName: observed.get.constructor && observed.get.constructor.name,
        cursorRequestIsMain: cursorRequest instanceof IDBRequest,
        cursorSourceIsIndex: cursorRequest.source === index,
        cursorIsMain: observed.cursor instanceof IDBCursor,
        cursorRequestMatches: observed.cursor.request === cursorRequest,
        cursorSourceMatches: observed.cursor.source === index
      });
    };

    getRequest.onerror = () => {
      globalThis.__indexedDbCrossRealmMethodResult =
        `get-error:${getRequest.error && getRequest.error.name}`;
    };
    getRequest.onsuccess = () => {
      observed.get = getRequest.result;
      finish();
    };
    cursorRequest.onerror = () => {
      globalThis.__indexedDbCrossRealmMethodResult =
        `cursor-error:${cursorRequest.error && cursorRequest.error.name}`;
    };
    cursorRequest.onsuccess = () => {
      observed.cursor = cursorRequest.result;
      finish();
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("cross-realm IndexedDB method workflow should schedule");

    for _ in 0..32 {
        if vm
            .eval("String(globalThis.__indexedDbCrossRealmMethodResult !== 'pending')")
            .expect("cross-realm IndexedDB method state should evaluate")
            == "true"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("cross-realm IndexedDB method workflow should advance");
    }

    assert_eq!(
        vm.eval("String(globalThis.__indexedDbCrossRealmMethodResult)")
            .expect("cross-realm IndexedDB method result should evaluate"),
        r#"{"getRequestIsMain":true,"getSourceIsStore":true,"getTransactionIsTransaction":true,"getResultUsesMainObjectPrototype":true,"getResultPrototypeIsNull":false,"getResultConstructorIsMain":true,"getResultConstructorName":"Object","cursorRequestIsMain":true,"cursorSourceIsIndex":true,"cursorIsMain":true,"cursorRequestMatches":true,"cursorSourceMatches":true}"#
    );
}

#[test]
fn indexed_db_open_cursor_captures_query_during_webidl_call() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-cursor-query-conversion.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbCursorQueryConversionResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("records");
    store.createIndex("byTag", "tag");
    store.put({name: "first", tag: ["alpha"]}, ["first"]);
    store.put({name: "second", tag: ["beta"]}, ["second"]);
  };
  open.onsuccess = () => {
    const blocker = open.result.transaction("records", "readwrite");
    blocker.objectStore("records");
    const tx = open.result.transaction("records", "readwrite");
    const store = tx.objectStore("records");
    const results = {};
    const finish = () => {
      if ("store" in results && "index" in results) {
        globalThis.__indexedDbCursorQueryConversionResult = JSON.stringify(results);
      }
    };

    const storeQuery = ["first"];
    const storeRequest = store.openCursor(storeQuery);
    storeQuery[0] = "second";
    storeRequest.onsuccess = () => {
      results.store = storeRequest.result && storeRequest.result.value.name;
      finish();
    };

    const indexQuery = ["alpha"];
    const indexRequest = store.index("byTag").openCursor(indexQuery);
    indexQuery[0] = "beta";
    indexRequest.onsuccess = () => {
      results.index = indexRequest.result && indexRequest.result.value.name;
      finish();
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb cursor query conversion workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbCursorQueryConversionResult)")
        .expect("indexeddb cursor query conversion result should be readable");

    assert_eq!(result, r#"{"store":"first","index":"first"}"#);
}

#[test]
fn indexed_db_object_store_cursors_can_iterate() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-store-cursor.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbStoreCursorResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => open.result.createObjectStore("kv");
  open.onsuccess = () => {
    const db = open.result;
    const writeTx = db.transaction("kv", "readwrite");
    const store = writeTx.objectStore("kv");
    store.put("one", "a");
    store.put("two", "b");
    store.put("three", "c");
    writeTx.oncomplete = () => {
      const tx = db.transaction("kv");
      const store = tx.objectStore("kv");
      const seen = [];
      let cursorKeys = "";
      const req = store.openCursor();
      req.onsuccess = () => {
        const cursor = req.result;
        if (!cursor) {
          const keyReq = store.openKeyCursor(IDBKeyRange.lowerBound("b"));
          keyReq.onsuccess = () => {
            const keyCursor = keyReq.result;
            const keyCursorKeys = Object.keys(keyCursor).sort().join(",");
            globalThis.__indexedDbStoreCursorResult = [
              seen.join(","),
              cursorKeys,
              keyCursor.key,
              keyCursor.primaryKey,
              keyCursorKeys
            ].join("|");
          };
          return;
        }
        if (cursorKeys === "") {
          cursorKeys = Object.keys(cursor).sort().join(",");
        }
        seen.push(`${cursor.key}:${cursor.value}`);
        if (cursor.key === "a") {
          cursor.advance(2);
        } else {
          cursor.continue();
        }
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb object store cursor workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbStoreCursorResult)")
        .expect("indexeddb object store cursor result should be readable");

    assert_eq!(
        result,
        "a:one,c:three|direction,key,primaryKey,request,source,value|b|b|direction,key,primaryKey,request,source"
    );
}

#[tokio::test]
async fn indexed_db_open_key_cursor_transaction_completes_after_iteration() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://indexeddb-open-key-cursor-complete.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbOpenKeyCursorComplete = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("store");
    for (let i = 0; i < 10; ++i) {
      store.put(`value: ${i}`, i);
    }
  };
  open.onerror = () => {
    globalThis.__indexedDbOpenKeyCursorComplete = `open-error:${open.error && open.error.name}`;
  };
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("store", "readonly");
    const store = tx.objectStore("store");
    const expected = "0,1,2,3,4,5,6,7,8,9";
    const actual = [];
    const request = store.openKeyCursor();
    request.onerror = () => {
      globalThis.__indexedDbOpenKeyCursorComplete = `request-error:${request.error && request.error.name}`;
    };
    request.onsuccess = () => {
      const cursor = request.result;
      if (!cursor)
        return;
      actual.push(cursor.key);
      cursor.continue();
    };
    tx.onabort = () => {
      globalThis.__indexedDbOpenKeyCursorComplete = `abort:${tx.error && tx.error.name}`;
    };
    tx.oncomplete = () => {
      globalThis.__indexedDbOpenKeyCursorComplete = `${actual.join(",")}|${expected}`;
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb openKeyCursor completion workflow should schedule");

    for _ in 0..32 {
        if vm
            .eval("String(globalThis.__indexedDbOpenKeyCursorComplete !== 'pending')")
            .expect("indexeddb openKeyCursor completion state should evaluate")
            == "true"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("indexeddb openKeyCursor completion should advance");
    }

    assert_eq!(
        vm.eval("String(globalThis.__indexedDbOpenKeyCursorComplete)")
            .expect("indexeddb openKeyCursor completion result should be readable"),
        "0,1,2,3,4,5,6,7,8,9|0,1,2,3,4,5,6,7,8,9"
    );
}

#[tokio::test]
async fn indexed_db_child_open_key_cursor_transaction_complete_keeps_child_sender() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://indexeddb-child-open-key-cursor-complete.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbChildOpenKeyCursorComplete = "pending";
  addEventListener("message", event => {
    globalThis.__indexedDbChildOpenKeyCursorComplete = JSON.stringify({
      data: event.data,
      sourceIsChild: event.source === frame.contentWindow
    });
  });
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <script>
    const open = indexedDB.open("app", 1);
    open.onupgradeneeded = () => {
      const store = open.result.createObjectStore("store");
      for (let i = 0; i < 10; ++i) {
        store.put("value: " + i, i);
      }
    };
    open.onsuccess = () => {
      const db = open.result;
      const tx = db.transaction("store", "readonly");
      const actual = [];
      const request = tx.objectStore("store").openKeyCursor();
      request.onsuccess = () => {
        const cursor = request.result;
        if (!cursor)
          return;
        actual.push(cursor.key);
        cursor.continue();
      };
      tx.oncomplete = () => {
        parent.postMessage({ kind: "complete", actual }, "*");
      };
    };
    </` + `script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.frame = frame;
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb child openKeyCursor completion workflow should schedule");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__indexedDbChildOpenKeyCursorComplete !== 'pending')",
        "true",
        "child IndexedDB cursor completion should arrive through selected Page tasks",
    )
    .await;

    assert_eq!(
        vm.eval("String(globalThis.__indexedDbChildOpenKeyCursorComplete)")
            .expect("indexeddb child openKeyCursor completion result should be readable"),
        r#"{"data":{"kind":"complete","actual":[0,1,2,3,4,5,6,7,8,9]},"sourceIsChild":true}"#
    );
}

#[test]
fn indexed_db_cursor_advance_and_continue_update_surface_on_next_success() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-cursor-async-surface.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbCursorAsyncSurfaceResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => open.result.createObjectStore("kv");
  open.onsuccess = () => {
    const db = open.result;
    const writeTx = db.transaction("kv", "readwrite");
    const store = writeTx.objectStore("kv");
    store.put("data", 1);
    store.put("data2", 2);
    writeTx.oncomplete = () => {
      const req = db.transaction("kv").objectStore("kv").openCursor();
      const seen = [];
      let count = 0;
      req.onsuccess = () => {
        const cursor = req.result;
        if (!cursor) {
          seen.push(`done:${count}`);
          globalThis.__indexedDbCursorAsyncSurfaceResult = seen.join("|");
          return;
        }
        if (count === 0) {
          cursor.continue();
          seen.push(`continue:${cursor.key}:${cursor.value}`);
        } else if (count === 1) {
          cursor.advance(1);
          seen.push(`advance:${cursor.key}:${cursor.value}`);
        }
        count++;
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb cursor async surface workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbCursorAsyncSurfaceResult)")
        .expect("indexeddb cursor async surface result should be readable");

    assert_eq!(result, "continue:1:data|advance:2:data2|done:2");
}

#[tokio::test]
async fn indexed_db_transaction_stays_active_through_creation_task_microtasks() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://indexeddb-creation-microtasks-active.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbCreationMicrotasksResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv").put("value", 1);
  };
  open.onsuccess = () => {
    const tx = open.result.transaction("kv");
    const store = tx.objectStore("kv");
    queueMicrotask(() => {
      queueMicrotask(() => {
        try {
          const request = store.get(1);
          request.onsuccess = () => {
            globalThis.__indexedDbCreationMicrotasksResult = `success:${request.result}`;
          };
          request.onerror = () => {
            globalThis.__indexedDbCreationMicrotasksResult = `request-error:${request.error && request.error.name}`;
          };
        } catch (error) {
          globalThis.__indexedDbCreationMicrotasksResult = `throw:${error && error.name}`;
        }
      });
    });
  };
  return "scheduled";
})()
"#,
    )
    .expect("IndexedDB creation-task microtask workflow should schedule");

    for _ in 0..8 {
        if vm
            .eval("String(globalThis.__indexedDbCreationMicrotasksResult !== 'pending')")
            .expect("IndexedDB creation-task microtask state should evaluate")
            == "true"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("IndexedDB creation-task microtask workflow should advance");
    }

    let result = vm
        .eval("String(globalThis.__indexedDbCreationMicrotasksResult)")
        .expect("IndexedDB creation-task microtask result should be readable");

    assert_eq!(result, "success:value");
}

#[tokio::test]
async fn indexed_db_object_store_open_key_cursor_rejects_after_transaction_inactive_task() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://indexeddb-open-key-cursor-inactive.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbInactiveOpenKeyCursorResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("kv");
    store.put("data", 1);
  };
  open.onsuccess = () => {
    const tx = open.result.transaction("kv");
    const store = tx.objectStore("kv");
    setTimeout(() => {
      try {
        store.openKeyCursor();
        globalThis.__indexedDbInactiveOpenKeyCursorResult = "no throw";
      } catch (error) {
        globalThis.__indexedDbInactiveOpenKeyCursorResult = error && error.name;
      }
    }, 0);
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb inactive openKeyCursor workflow should schedule");

    for _ in 0..8 {
        if vm
            .eval("String(globalThis.__indexedDbInactiveOpenKeyCursorResult !== 'pending')")
            .expect("indexeddb inactive openKeyCursor state should evaluate")
            == "true"
        {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
            .await
            .expect("indexeddb inactive openKeyCursor should advance");
    }

    let result = vm
        .eval("String(globalThis.__indexedDbInactiveOpenKeyCursorResult)")
        .expect("indexeddb inactive openKeyCursor result should be readable");

    assert_eq!(result, "TransactionInactiveError");
}

#[test]
fn indexed_db_cursor_advance_conversion_matches_wpt() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-advance-conversion.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbAdvanceConversionResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
    const db = open.result;
    const writeTx = db.transaction("kv", "readwrite");
    const store = writeTx.objectStore("kv");
    store.put("one", "a");
    store.put("two", "b");
    store.put("three", "c");
    writeTx.oncomplete = () => {
      const outputs = [];
      const cases = [
        ["missing", (cursor) => cursor.advance()],
        ["undefined", (cursor) => cursor.advance(undefined)],
        ["null", (cursor) => cursor.advance(null)],
        ["NaN", (cursor) => cursor.advance(NaN)],
        ["fraction", (cursor) => cursor.advance(1.9)],
        ["string", (cursor) => cursor.advance("2")],
        ["negative", (cursor) => cursor.advance(-1)],
        ["zero", (cursor) => cursor.advance(0)],
        ["symbol", (cursor) => cursor.advance(Symbol())],
      ];

      function runCase(index) {
        if (index >= cases.length) {
          runExhaustedCase();
          return;
        }
        const [label, run] = cases[index];
        const req = db.transaction("kv", "readonly").objectStore("kv").openCursor();
        req.onsuccess = () => {
          const cursor = req.result;
          req.onsuccess = null;
          try {
            run(cursor);
            outputs.push(`${label}:ok`);
          } catch (error) {
            outputs.push(`${label}:${error.name}`);
          }
          runCase(index + 1);
        };
      }

      function runExhaustedCase() {
        const req = db.transaction("kv", "readonly").objectStore("kv").openCursor();
        let heldCursor = null;
        req.onsuccess = () => {
          if (req.result) {
            heldCursor = req.result;
            heldCursor.continue();
            return;
          }
          try {
            heldCursor.advance(1);
            outputs.push("exhausted:ok");
          } catch (error) {
            outputs.push(`exhausted:${error.name}`);
          }
          globalThis.__indexedDbAdvanceConversionResult = outputs.join("|");
        };
      }

      runCase(0);
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb advance conversion workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbAdvanceConversionResult)")
        .expect("indexeddb advance conversion result should be readable");

    assert_eq!(
        result,
        "missing:TypeError|undefined:TypeError|null:TypeError|NaN:TypeError|fraction:ok|string:ok|negative:TypeError|zero:TypeError|symbol:TypeError|exhausted:InvalidStateError"
    );
}

#[test]
fn indexed_db_index_cursors_can_iterate() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-index-cursor.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbIndexCursorResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("posts", { keyPath: "id" });
    store.createIndex("by-tag", "tag");
  };
  open.onsuccess = () => {
    const db = open.result;
    const writeTx = db.transaction("posts", "readwrite");
    const store = writeTx.objectStore("posts");
    store.put({ id: "a", tag: "news", value: 1 });
    store.put({ id: "b", tag: "news", value: 2 });
    store.put({ id: "c", tag: "tech", value: 3 });
    writeTx.oncomplete = () => {
      const index = db.transaction("posts").objectStore("posts").index("by-tag");
      const seen = [];
      const req = index.openCursor(IDBKeyRange.only("news"));
      req.onsuccess = () => {
        const cursor = req.result;
        if (!cursor) {
          const keyReq = index.openKeyCursor(IDBKeyRange.only("news"));
          keyReq.onsuccess = () => {
            const keyCursor = keyReq.result;
            globalThis.__indexedDbIndexCursorResult = [
              seen.join(","),
              keyCursor.key,
              keyCursor.primaryKey
            ].join("|");
          };
          return;
        }
        seen.push(`${cursor.key}:${cursor.primaryKey}:${cursor.value.value}`);
        cursor.continue();
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb index cursor workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbIndexCursorResult)")
        .expect("indexeddb index cursor result should be readable");

    assert_eq!(result, "news:a:1,news:b:2|news|a");
}

#[test]
fn indexed_db_index_cursor_continue_primary_key_and_unique_direction_work() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-index-cursor-primary.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbIndexCursorPrimaryResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("posts", { keyPath: "id" });
    store.createIndex("by-tag", "tag");
  };
  open.onsuccess = () => {
    const db = open.result;
    const writeTx = db.transaction("posts", "readwrite");
    const store = writeTx.objectStore("posts");
    store.put({ id: "a", tag: "news", value: 1 });
    store.put({ id: "b", tag: "news", value: 2 });
    store.put({ id: "c", tag: "tech", value: 3 });
    writeTx.oncomplete = () => {
      const index = db.transaction("posts").objectStore("posts").index("by-tag");
      const req = index.openCursor();
      req.onsuccess = () => {
        const cursor = req.result;
        if (!cursor) return;
        cursor.continuePrimaryKey("news", "b");
        req.onsuccess = () => {
          const cursor2 = req.result;
          const uniqueReq = index.openKeyCursor(undefined, "nextunique");
          uniqueReq.onsuccess = () => {
            const uniqueCursor = uniqueReq.result;
            globalThis.__indexedDbIndexCursorPrimaryResult = [
              cursor2 && cursor2.key,
              cursor2 && cursor2.primaryKey,
              uniqueCursor && uniqueCursor.key,
              uniqueCursor && uniqueCursor.primaryKey
            ].join("|");
          };
        };
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb index cursor continuePrimaryKey workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbIndexCursorPrimaryResult)")
        .expect("indexeddb index cursor continuePrimaryKey result should be readable");

    assert_eq!(result, "news|b|news|a");
}

#[test]
fn indexed_db_object_store_cursor_update_and_delete_work() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-store-cursor-write.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbStoreCursorWriteResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => open.result.createObjectStore("kv");
  open.onsuccess = () => {
    const db = open.result;
    const writeTx = db.transaction("kv", "readwrite");
    const store = writeTx.objectStore("kv");
    store.put({ value: "one" }, "a");
    store.put({ value: "two" }, "b");
    writeTx.oncomplete = () => {
      const tx = db.transaction("kv", "readwrite");
      const store = tx.objectStore("kv");
      const req = store.openCursor();
      req.onsuccess = () => {
        const cursor = req.result;
        if (!cursor) return;
        const updateReq = cursor.update({ value: "ONE" });
        updateReq.onsuccess = () => {
          cursor.continue();
          req.onsuccess = () => {
            const cursor2 = req.result;
            const deleteReq = cursor2.delete();
            deleteReq.onsuccess = () => {
              tx.oncomplete = () => {
                const readTx = db.transaction("kv");
                const readStore = readTx.objectStore("kv");
                const getA = readStore.get("a");
                getA.onsuccess = () => {
                  const getB = readStore.get("b");
                  getB.onsuccess = () => {
                    globalThis.__indexedDbStoreCursorWriteResult = [
                      getA.result.value,
                      String(getB.result)
                    ].join("|");
                  };
                };
              };
            };
          };
        };
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb object store cursor write workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbStoreCursorWriteResult)")
        .expect("indexeddb object store cursor write result should be readable");

    assert_eq!(result, "ONE|undefined");
}

#[test]
fn indexed_db_index_cursor_update_and_delete_work() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-index-cursor-write.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbIndexCursorWriteResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("posts", { keyPath: "id" });
    store.createIndex("by-tag", "tag");
  };
  open.onsuccess = () => {
    const db = open.result;
    const writeTx = db.transaction("posts", "readwrite");
    const store = writeTx.objectStore("posts");
    store.put({ id: "a", tag: "news", value: 1 });
    store.put({ id: "b", tag: "news", value: 2 });
    writeTx.oncomplete = () => {
      const tx = db.transaction("posts", "readwrite");
      const index = tx.objectStore("posts").index("by-tag");
      const req = index.openCursor(IDBKeyRange.only("news"));
      req.onsuccess = () => {
        const cursor = req.result;
        if (!cursor) return;
        const updateReq = cursor.update({ id: "a", tag: "news", value: 10 });
        updateReq.onsuccess = () => {
          cursor.continue();
          req.onsuccess = () => {
            const cursor2 = req.result;
            const deleteReq = cursor2.delete();
            deleteReq.onsuccess = () => {
              tx.oncomplete = () => {
                const readStore = db.transaction("posts").objectStore("posts");
                const getA = readStore.get("a");
                getA.onsuccess = () => {
                  const getB = readStore.get("b");
                  getB.onsuccess = () => {
                    globalThis.__indexedDbIndexCursorWriteResult = [
                      getA.result.value,
                      String(getB.result)
                    ].join("|");
                  };
                };
              };
            };
          };
        };
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb index cursor write workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbIndexCursorWriteResult)")
        .expect("indexeddb index cursor write result should be readable");

    assert_eq!(result, "10|undefined");
}

#[test]
fn indexed_db_cursors_support_prev_directions() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-cursor-prev.test/");

    vm.eval(
            r#"
(() => {
  globalThis.__indexedDbCursorPrevResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("posts", { keyPath: "id" });
    store.createIndex("by-tag", "tag");
  };
  open.onsuccess = () => {
    const db = open.result;
    const writeTx = db.transaction("posts", "readwrite");
    const store = writeTx.objectStore("posts");
    store.put({ id: "a", tag: "news" });
    store.put({ id: "b", tag: "news" });
    store.put({ id: "c", tag: "tech" });
    writeTx.oncomplete = () => {
      const storeReq = db.transaction("posts").objectStore("posts").openKeyCursor(null, "prev");
      const seenStore = [];
      storeReq.onsuccess = () => {
        const cursor = storeReq.result;
        if (!cursor) {
          const indexReq = db.transaction("posts").objectStore("posts").index("by-tag").openKeyCursor(undefined, "prevunique");
          const seenIndex = [];
          indexReq.onsuccess = () => {
            const indexCursor = indexReq.result;
            if (!indexCursor) {
              globalThis.__indexedDbCursorPrevResult = [
                seenStore.join(","),
                seenIndex.join(",")
              ].join("|");
              return;
            }
            seenIndex.push(`${indexCursor.key}:${indexCursor.primaryKey}`);
            indexCursor.continue();
          };
          return;
        }
        seenStore.push(cursor.primaryKey);
        cursor.continue();
      };
    };
  };
  return "scheduled";
})()
"#,
        )
        .expect("indexeddb cursor prev workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbCursorPrevResult)")
        .expect("indexeddb cursor prev result should be readable");

    assert_eq!(result, "c,b,a|tech:c,news:b");
}

#[test]
fn indexed_db_cursor_continue_validates_target_position() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-cursor-continue-error.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbCursorContinueError = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => open.result.createObjectStore("kv");
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("kv", "readwrite");
    tx.objectStore("kv").put("one", "a");
    tx.oncomplete = () => {
      const req = db.transaction("kv").objectStore("kv").openCursor();
      req.onsuccess = () => {
        try {
          req.result.continue("a");
          globalThis.__indexedDbCursorContinueError = "no-error";
        } catch (error) {
          globalThis.__indexedDbCursorContinueError = error.name;
        }
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb cursor continue error workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbCursorContinueError)")
        .expect("indexeddb cursor continue error result should be readable");

    assert_eq!(result, "DataError");
}

#[test]
fn indexed_db_cursor_continue_primary_key_validates_target_position() {
    let mut vm = new_storage_page_task_executor_test_vm(
        "https://indexeddb-cursor-continue-primary-error.test/",
    );

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbCursorContinuePrimaryError = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("posts", { keyPath: "id" });
    store.createIndex("by-tag", "tag");
  };
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("posts", "readwrite");
    const store = tx.objectStore("posts");
    store.put({ id: "a", tag: "news" });
    store.put({ id: "b", tag: "news" });
    tx.oncomplete = () => {
      const req = db.transaction("posts").objectStore("posts").index("by-tag").openCursor();
      req.onsuccess = () => {
        try {
          req.result.continuePrimaryKey("news", "a");
          globalThis.__indexedDbCursorContinuePrimaryError = "no-error";
        } catch (error) {
          globalThis.__indexedDbCursorContinuePrimaryError = error.name;
        }
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb cursor continuePrimaryKey error workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbCursorContinuePrimaryError)")
        .expect("indexeddb cursor continuePrimaryKey error result should be readable");

    assert_eq!(result, "DataError");
}

#[test]
fn indexed_db_cursor_update_and_delete_fail_in_readonly_transactions() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-cursor-readonly-error.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbCursorReadonlyError = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => open.result.createObjectStore("kv");
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("kv", "readwrite");
    tx.objectStore("kv").put({ value: 1 }, "a");
    tx.oncomplete = () => {
      const req = db.transaction("kv").objectStore("kv").openCursor();
      req.onsuccess = () => {
        const cursor = req.result;
        const updateReq = cursor.update({ value: 2 });
        updateReq.onerror = () => {
          const deleteReq = cursor.delete();
          deleteReq.onerror = () => {
            globalThis.__indexedDbCursorReadonlyError = [
              updateReq.error && updateReq.error.name,
              deleteReq.error && deleteReq.error.name
            ].join("|");
          };
        };
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb cursor readonly error workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbCursorReadonlyError)")
        .expect("indexeddb cursor readonly error result should be readable");

    assert_eq!(result, "ReadOnlyError|ReadOnlyError");
}

#[test]
fn indexed_db_abort_converts_pending_request_into_abort_error_and_rolls_back() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-transaction-abort.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbAbortResult = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => open.result.createObjectStore("kv");
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("kv", "readwrite");
    const store = tx.objectStore("kv");
    const putReq = store.put("one", "a");
    let requestError = "pending";
    putReq.onerror = () => {
      requestError = putReq.error && putReq.error.name;
    };
    tx.onabort = () => {
      const readReq = db.transaction("kv").objectStore("kv").get("a");
      readReq.onsuccess = () => {
        globalThis.__indexedDbAbortResult = [
          requestError,
          tx.error && tx.error.name,
          String(readReq.result)
        ].join("|");
      };
    };
    tx.abort();
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb abort workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbAbortResult)")
        .expect("indexeddb abort result should be readable");

    assert_eq!(result, "AbortError|AbortError|undefined");
}

#[test]
fn indexed_db_aborted_upgrade_request_errors_with_abort_error() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-upgrade-abort.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbUpgradeAbortResult = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
    open.transaction.abort();
  };
  open.onerror = () => {
    globalThis.__indexedDbUpgradeAbortResult = open.error && open.error.name;
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb upgrade abort workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbUpgradeAbortResult)")
        .expect("indexeddb upgrade abort result should be readable");

    assert_eq!(result, "AbortError");
}

#[test]
fn indexed_db_aborted_upgrade_closes_provisional_connection_before_reopen_and_delete() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-upgrade-abort-close.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbUpgradeAbortCloseResult = "pending";
  const dbName = `app-${Math.random()}`;
  const initial = indexedDB.open(dbName, 1);
  initial.onupgradeneeded = () => initial.result.createObjectStore("records");
  initial.onsuccess = () => {
    initial.result.close();
    const upgrade = indexedDB.open(dbName, 2);
    upgrade.onupgradeneeded = () => {
      upgrade.result.createObjectStore("transient");
      upgrade.transaction.abort();
    };
    upgrade.onerror = () => {
      const errorName = upgrade.error && upgrade.error.name;
      const reopen = indexedDB.open(dbName);
      reopen.onsuccess = () => {
        const database = reopen.result;
        const snapshot = `${errorName}|${database.version}|${Array.from(database.objectStoreNames).join(",")}`;
        database.close();
        const deletion = indexedDB.deleteDatabase(dbName);
        deletion.onblocked = () => {
          globalThis.__indexedDbUpgradeAbortCloseResult = `${snapshot}|blocked`;
        };
        deletion.onsuccess = () => {
          globalThis.__indexedDbUpgradeAbortCloseResult = `${snapshot}|deleted`;
        };
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb aborted upgrade cleanup workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbUpgradeAbortCloseResult)")
        .expect("indexeddb aborted upgrade cleanup result should be readable");

    assert_eq!(result, "AbortError|1|records|deleted");
}

#[test]
fn indexed_db_upgrade_open_dispatches_versionchange_and_blocked_until_close() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-open-blocked.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbBlockedOpenResult = "pending";
  const dbName = `app-${Math.random()}`;
  const first = indexedDB.open(dbName, 1);
  first.onupgradeneeded = () => first.result.createObjectStore("v1");
  first.onsuccess = () => {
    const db1 = first.result;
    let versionChange = "none";
    let blocked = false;
    db1.onversionchange = (event) => {
      versionChange = `${event.oldVersion}|${event.newVersion}`;
    };
    const second = indexedDB.open(dbName, 2);
    second.onblocked = () => {
      blocked = true;
      db1.close();
    };
    second.onupgradeneeded = () => {
      second.result.createObjectStore("v2");
    };
    second.onerror = () => {
      globalThis.__indexedDbBlockedOpenResult = `error:${second.error && second.error.name}`;
    };
    second.onsuccess = () => {
      globalThis.__indexedDbBlockedOpenResult = [
        versionChange,
        String(blocked),
        second.result.version,
        second.result.objectStoreNames.contains("v2")
      ].join("|");
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb blocked open workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbBlockedOpenResult)")
        .expect("indexeddb blocked open result should be readable");

    assert_eq!(result, "1|2|true|2|true");
}

#[test]
fn indexed_db_delete_database_dispatches_versionchange_and_blocked_until_close() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-delete-blocked.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbDeleteBlockedResult = "pending";
  const dbName = `app-${Math.random()}`;
  const first = indexedDB.open(dbName, 1);
  first.onupgradeneeded = () => first.result.createObjectStore("v1");
  first.onsuccess = () => {
    const db1 = first.result;
    let versionChange = "none";
    let blocked = false;
    db1.onversionchange = (event) => {
      versionChange = `${event.oldVersion}|${String(event.newVersion)}`;
    };
    const del = indexedDB.deleteDatabase(dbName);
    del.onblocked = () => {
      blocked = true;
      db1.close();
    };
    del.onerror = () => {
      globalThis.__indexedDbDeleteBlockedResult = `delete-error:${del.error && del.error.name}`;
    };
    del.onsuccess = () => {
      let recreated = "none";
      const reopen = indexedDB.open(dbName, 1);
      reopen.onupgradeneeded = (event) => {
        recreated = `${event.oldVersion}|${event.newVersion}`;
        reopen.result.createObjectStore("fresh");
      };
      reopen.onsuccess = () => {
        globalThis.__indexedDbDeleteBlockedResult = [
          versionChange,
          String(blocked),
          recreated,
          reopen.result.objectStoreNames.contains("fresh")
        ].join("|");
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb delete blocked workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbDeleteBlockedResult)")
        .expect("indexeddb delete blocked result should be readable");

    assert_eq!(result, "1|null|true|0|1|true");
}

#[test]
fn indexed_db_delete_database_queues_while_initial_upgrade_connection_is_open() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-delete-upgrade-queue.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbDeleteUpgradeQueueResult = "pending";
  const dbName = `app-${Math.random()}`;
  const saw = [];
  const maybeFinish = () => {
    if (saw.length === 2) {
      globalThis.__indexedDbDeleteUpgradeQueueResult = saw.join("|");
    }
  };
  const open = indexedDB.open(dbName, 1);
  open.onerror = () => {
    globalThis.__indexedDbDeleteUpgradeQueueResult = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    const firstDelete = indexedDB.deleteDatabase(open.result.name);
    firstDelete.onerror = () => {
      globalThis.__indexedDbDeleteUpgradeQueueResult = `delete1-error:${firstDelete.error && firstDelete.error.name}`;
    };
    firstDelete.onsuccess = () => {
      saw.push("delete1");
      maybeFinish();
    };
  };
  open.onsuccess = () => {
    const secondDelete = indexedDB.deleteDatabase(open.result.name);
    secondDelete.onerror = () => {
      globalThis.__indexedDbDeleteUpgradeQueueResult = `delete2-error:${secondDelete.error && secondDelete.error.name}`;
    };
    secondDelete.onsuccess = () => {
      saw.push("delete2");
      maybeFinish();
    };
    open.result.close();
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb delete upgrade queue workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbDeleteUpgradeQueueResult)")
        .expect("indexeddb delete upgrade queue result should be readable");

    assert_eq!(result, "delete1|delete2");
}

#[test]
fn indexed_db_concurrent_readwrite_transactions_are_serialized() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-readwrite-queue.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbReadwriteQueueResult = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => open.result.createObjectStore("kv");
  open.onsuccess = () => {
    const db = open.result;
    const completed = [];
    const tx1 = db.transaction("kv", "readwrite");
    tx1.objectStore("kv").put("one", "a");
    tx1.oncomplete = () => {
      completed.push("tx1");
    };
    const tx2 = db.transaction("kv", "readwrite");
    tx2.objectStore("kv").put("two", "b");
    tx2.oncomplete = () => {
      completed.push("tx2");
      const read = db.transaction("kv");
      const store = read.objectStore("kv");
      const getA = store.get("a");
      getA.onsuccess = () => {
        const getB = store.get("b");
        getB.onsuccess = () => {
          globalThis.__indexedDbReadwriteQueueResult = [
            completed.join(","),
            getA.result,
            getB.result
          ].join("|");
        };
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb readwrite queue workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbReadwriteQueueResult)")
        .expect("indexeddb readwrite queue result should be readable");

    assert_eq!(result, "tx1,tx2|one|two");
}

#[test]
fn indexed_db_queued_readwrite_requests_preserve_write_then_read_order() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-readwrite-queued-reads.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbQueuedReadResult = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => open.result.createObjectStore("kv");
  open.onsuccess = () => {
    const db = open.result;
    const tx1 = db.transaction("kv", "readwrite");
    tx1.objectStore("kv").put("one", "a");

    const tx2 = db.transaction("kv", "readwrite");
    const store = tx2.objectStore("kv");
    store.put("two", "b");
    const getReq = store.get("b");
    const countReq = store.count();
    const cursorSeen = [];
    const cursorReq = store.openCursor();
    let getValue = "missing";
    let countValue = "missing";
    let cursorValue = "missing";

    getReq.onsuccess = () => {
      getValue = String(getReq.result);
    };
    countReq.onsuccess = () => {
      countValue = String(countReq.result);
    };
    cursorReq.onsuccess = () => {
      const cursor = cursorReq.result;
      if (!cursor) {
        cursorValue = cursorSeen.join(",");
        return;
      }
      cursorSeen.push(`${cursor.key}:${cursor.value}`);
      cursor.continue();
    };
    tx2.oncomplete = () => {
      globalThis.__indexedDbQueuedReadResult = [
        getValue,
        countValue,
        cursorValue
      ].join("|");
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb queued readwrite read workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbQueuedReadResult)")
        .expect("indexeddb queued readwrite read result should be readable");

    assert_eq!(result, "two|2|a:one,b:two");
}

#[test]
fn indexed_db_queued_readwrite_index_requests_run_after_transaction_starts() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-readwrite-queued-index.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbQueuedIndexResult = "pending";
  const dbName = `app-${Math.random()}`;
  const open = indexedDB.open(dbName, 1);
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("posts", { keyPath: "id" });
    store.createIndex("by-tag", "tag");
  };
  open.onsuccess = () => {
    const db = open.result;
    const tx1 = db.transaction("posts", "readwrite");
    tx1.objectStore("posts").put({ id: "a", tag: "news", value: 1 });

    const tx2 = db.transaction("posts", "readwrite");
    const store = tx2.objectStore("posts");
    store.put({ id: "b", tag: "news", value: 2 });
    const index = store.index("by-tag");
    const getReq = index.get("news");
    const allReq = index.getAll(IDBKeyRange.only("news"));
    const countReq = index.count("news");
    const cursorSeen = [];
    const cursorReq = index.openCursor(IDBKeyRange.only("news"));
    let getValue = "missing";
    let allValue = "missing";
    let countValue = "missing";
    let cursorValue = "missing";

    getReq.onsuccess = () => {
      getValue = getReq.result && getReq.result.id;
    };
    allReq.onsuccess = () => {
      allValue = allReq.result.map((item) => item.id).join(",");
    };
    countReq.onsuccess = () => {
      countValue = String(countReq.result);
    };
    cursorReq.onsuccess = () => {
      const cursor = cursorReq.result;
      if (!cursor) {
        cursorValue = cursorSeen.join(",");
        return;
      }
      cursorSeen.push(`${cursor.key}:${cursor.primaryKey}`);
      cursor.continue();
    };
    tx2.oncomplete = () => {
      globalThis.__indexedDbQueuedIndexResult = [
        getValue,
        allValue,
        countValue,
        cursorValue
      ].join("|");
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb queued readwrite index workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbQueuedIndexResult)")
        .expect("indexeddb queued readwrite index result should be readable");

    assert_eq!(result, "a|a,b|2|news:a,news:b");
}

#[test]
fn indexed_db_cursor_update_rejects_keypath_key_change() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-cursor-keypath-error.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbCursorKeyPathError = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => open.result.createObjectStore("posts", { keyPath: "id" });
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("posts", "readwrite");
    tx.objectStore("posts").put({ id: "a", value: 1 });
    tx.oncomplete = () => {
      const cursorReq = db.transaction("posts", "readwrite").objectStore("posts").openCursor();
      cursorReq.onsuccess = () => {
        try {
          cursorReq.result.update({ id: "b", value: 2 });
          globalThis.__indexedDbCursorKeyPathError = "no-error";
        } catch (error) {
          globalThis.__indexedDbCursorKeyPathError = error.name;
        }
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb cursor keypath error workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbCursorKeyPathError)")
        .expect("indexeddb cursor keypath error result should be readable");

    assert_eq!(result, "DataError");
}

#[test]
fn indexed_db_exhausted_cursor_throws_invalid_state() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-cursor-exhausted.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__indexedDbCursorExhaustedError = "pending";
  const open = indexedDB.open("app", 1);
  open.onupgradeneeded = () => open.result.createObjectStore("kv");
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("kv", "readwrite");
    tx.objectStore("kv").put("one", "a");
    tx.oncomplete = () => {
      const req = db.transaction("kv").objectStore("kv").openCursor();
      let cursorRef;
      req.onsuccess = () => {
        if (req.result) {
          cursorRef = req.result;
          req.result.continue();
          req.onsuccess = () => {
            try {
              cursorRef.continue();
              globalThis.__indexedDbCursorExhaustedError = "no-error";
            } catch (error) {
              globalThis.__indexedDbCursorExhaustedError = error.name;
            }
          };
        }
      };
    };
  };
  return "scheduled";
})()
"#,
    )
    .expect("indexeddb exhausted cursor workflow should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__indexedDbCursorExhaustedError)")
        .expect("indexeddb exhausted cursor result should be readable");

    assert_eq!(result, "InvalidStateError");
}

async fn spawn_indexed_db_databases_child_server() -> (String, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("[::1]:0")
        .await
        .expect("bind IndexedDB databases child server");
    let addr = listener
        .local_addr()
        .expect("IndexedDB databases child server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept IndexedDB databases child request");
        let request = read_indexed_db_databases_child_request_head(&mut stream)
            .await
            .expect("read IndexedDB databases child request");
        let status = if request.starts_with("GET /indexeddb-databases-child.html ") {
            "200 OK"
        } else {
            "404 Not Found"
        };
        let body = r#"<!doctype html>
<meta charset="utf-8">
<script>
addEventListener("message", async event => {
  let response;
  try {
    if (event.data && event.data.action === "delete") {
      await new Promise((resolve, reject) => {
        const request = indexedDB.deleteDatabase(event.data.name);
        request.onsuccess = resolve;
        request.onerror = reject;
      });
      response = { ok: true, deleted: true };
    } else {
      const infos = await indexedDB.databases();
      response = { ok: true, names: infos.map(info => info.name) };
    }
  } catch (error) {
    response = { ok: false, error: error && error.name };
  }
  event.source.postMessage(JSON.stringify(response), event.origin);
  window.close();
});
if (window.opener !== null) {
  window.opener.postMessage("ready", "*");
}
</script>
"#;
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        use tokio::io::AsyncWriteExt;
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write IndexedDB databases child response");
        request
    });
    (
        format!("http://{addr}/indexeddb-databases-child.html"),
        server,
    )
}

async fn spawn_indexed_db_origin_isolation_child_server()
-> (String, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("[::1]:0")
        .await
        .expect("bind IndexedDB origin isolation child server");
    let addr = listener
        .local_addr()
        .expect("IndexedDB origin isolation child server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept IndexedDB origin isolation child request");
        let request = read_indexed_db_databases_child_request_head(&mut stream)
            .await
            .expect("read IndexedDB origin isolation child request");
        let status = if request.starts_with("GET /indexeddb-origin-isolation-child.html ") {
            "200 OK"
        } else {
            "404 Not Found"
        };
        let body = r#"<!doctype html>
<meta charset="utf-8">
<script>
function keep_alive(tx, store_name) {
  let keepSpinning = true;
  function spin() {
    if (!keepSpinning)
      return;
    tx.objectStore(store_name).get(0).onsuccess = spin;
  }
  spin();
  return () => { keepSpinning = false; };
}
const request = indexedDB.open("shared-origin-lock-db", 1);
request.onupgradeneeded = () => {
  request.result.createObjectStore("s");
};
request.onerror = () => {
  parent.postMessage({ kind: "child-error", error: request.error && request.error.name }, "*");
};
request.onsuccess = () => {
  const tx = request.result.transaction("s", "readonly");
  keep_alive(tx, "s");
  parent.postMessage({ kind: "child-ready" }, "*");
};
</script>
"#;
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        use tokio::io::AsyncWriteExt;
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write IndexedDB origin isolation child response");
        request
    });
    (
        format!("http://{addr}/indexeddb-origin-isolation-child.html"),
        server,
    )
}

async fn read_indexed_db_databases_child_request_head(
    stream: &mut tokio::net::TcpStream,
) -> std::io::Result<String> {
    use tokio::io::AsyncReadExt;

    let mut buffer = Vec::new();
    let mut chunk = [0; 512];
    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}
