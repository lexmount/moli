use super::*;

#[test]
fn file_reader_read_as_text_symbol_encoding_throws_before_reading() {
    let mut vm = new_storage_test_vm("https://filereader-symbol-encoding.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const reader = new FileReader();
  let caught = false;
  try {
    reader.readAsText(new Blob(["x"]), Symbol("encoding"));
  } catch (error) {
    caught = error instanceof TypeError;
  }
  return JSON.stringify({
    caught,
    readyState: reader.readyState,
    result: reader.result,
    empty: FileReader.EMPTY
  });
})()
"#,
        )
        .expect("FileReader Symbol encoding probe should evaluate");

    assert_eq!(
        result,
        r#"{"caught":true,"readyState":0,"result":null,"empty":0}"#
    );
}
#[test]
fn file_reader_backing_state_is_not_own_property_surface() {
    let mut vm = new_storage_test_vm("https://filereader-private-state.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const constantDescriptor = (owner, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(owner, name);
    return [
      name,
      descriptor?.value,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable
    ].join(":");
  };
  const descriptorReport = name => {
    const descriptor = Object.getOwnPropertyDescriptor(FileReader.prototype, name);
    return [
      name,
      typeof descriptor?.get,
      descriptor?.get?.name,
      descriptor?.get?.length,
      typeof descriptor?.set,
      descriptor?.enumerable,
      descriptor?.configurable
    ].join(":");
  };
  const reader = new FileReader();
  const eventHandlerNames = [
    "onloadstart", "onprogress", "onload", "onabort", "onerror", "onloadend"
  ];
  const leaked = () => Object.getOwnPropertyNames(reader)
    .filter(name => name.startsWith("__lmFileReader") ||
                    name.startsWith("__moliFileReader"))
    .sort();
  const readyStateDescriptor = Object.getOwnPropertyDescriptor(FileReader.prototype, "readyState");
  const resultDescriptor = Object.getOwnPropertyDescriptor(FileReader.prototype, "result");
  const errorDescriptor = Object.getOwnPropertyDescriptor(FileReader.prototype, "error");
  const fake = {
    __lmFileReaderReadyState: 1,
    __lmFileReaderResult: "spoofed",
    __lmFileReaderError: "spoofed"
  };
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  const handler = () => {};
  reader.onload = handler;
  const handlerRoundTrips = reader.onload === handler;
  reader.onload = {};
  const nonCallableHandlerBecomesNull = reader.onload === null;
  const before = leaked();
  reader.addEventListener("load", () => {});
  reader.readAsText(new Blob(["abc"]));
  const during = leaked();
  return JSON.stringify({
    descriptors: [
      descriptorReport("readyState"),
      descriptorReport("result"),
      descriptorReport("error"),
      ...eventHandlerNames.map(descriptorReport)
    ],
    constructorConstants: ["EMPTY", "LOADING", "DONE"].map(name =>
      constantDescriptor(FileReader, name)
    ),
    prototypeConstants: ["EMPTY", "LOADING", "DONE"].map(name =>
      constantDescriptor(FileReader.prototype, name)
    ),
    readerOwnConstants: ["EMPTY", "LOADING", "DONE"].filter(name =>
      Object.prototype.hasOwnProperty.call(reader, name)
    ),
    readerOwnHandlers: eventHandlerNames.filter(name =>
      Object.prototype.hasOwnProperty.call(reader, name)
    ),
    before,
    during,
    resultIsNull: reader.result === null,
    handlerRoundTrips,
    nonCallableHandlerBecomesNull,
    fakeReadyStateThrows: throwsTypeError(() => readyStateDescriptor.get.call(fake)),
    fakeResultThrows: throwsTypeError(() => resultDescriptor.get.call(fake)),
    fakeErrorThrows: throwsTypeError(() => errorDescriptor.get.call(fake)),
    fakeAbortThrows: throwsTypeError(() => FileReader.prototype.abort.call(fake))
  });
})()
"#,
        )
        .expect("FileReader private state reflection probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":["readyState:function:get readyState:0:undefined:true:true","result:function:get result:0:undefined:true:true","error:function:get error:0:undefined:true:true","onloadstart:function:get onloadstart:0:function:true:true","onprogress:function:get onprogress:0:function:true:true","onload:function:get onload:0:function:true:true","onabort:function:get onabort:0:function:true:true","onerror:function:get onerror:0:function:true:true","onloadend:function:get onloadend:0:function:true:true"],"constructorConstants":["EMPTY:0:true:false:false","LOADING:1:true:false:false","DONE:2:true:false:false"],"prototypeConstants":["EMPTY:0:true:false:false","LOADING:1:true:false:false","DONE:2:true:false:false"],"readerOwnConstants":[],"readerOwnHandlers":[],"before":[],"during":[],"resultIsNull":true,"handlerRoundTrips":true,"nonCallableHandlerBecomesNull":true,"fakeReadyStateThrows":true,"fakeResultThrows":true,"fakeErrorThrows":true,"fakeAbortThrows":true}"#
    );
}
#[test]
fn file_reader_listeners_use_event_listener_callback_interface_semantics() {
    let mut vm = new_storage_test_vm("https://file-reader-callback-interface.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__fileReaderCallbackCalls = [];
  globalThis.__fileReaderOperationGets = 0;
  globalThis.__fileReaderCallableOperationGets = 0;
  globalThis.__fileReaderOnceCalls = 0;
  const reader = new FileReader();
  globalThis.__fileReaderCallbackTarget = reader;

  const objectListener = {};
  Object.defineProperty(objectListener, "handleEvent", {
    configurable: true,
    get() {
      __fileReaderOperationGets++;
      return function(event) {
        __fileReaderCallbackCalls.push(
          `object:${this === objectListener}:${event.currentTarget === reader}:${window.event === event}`
        );
      };
    }
  });
  reader.addEventListener("load", objectListener);
  reader.addEventListener("load", objectListener, { once: true });

  function callable(event) {
    "use strict";
    __fileReaderCallbackCalls.push(
      `callable:${this === reader}:${event.currentTarget === reader}`
    );
  }
  Object.defineProperty(callable, "handleEvent", {
    get() {
      __fileReaderCallableOperationGets++;
      throw new Error("the callable branch must not resolve handleEvent");
    }
  });
  reader.addEventListener("load", callable);

  const removedBeforeVisit = () => __fileReaderCallbackCalls.push("removed");
  const late = () => __fileReaderCallbackCalls.push("late");
  reader.addEventListener("load", () => {
    __fileReaderCallbackCalls.push("mutator");
    reader.removeEventListener("load", removedBeforeVisit);
    reader.addEventListener("load", late);
  });
  reader.addEventListener("load", removedBeforeVisit);

  const once = () => {
    __fileReaderOnceCalls++;
    reader.addEventListener("load", once, { once: true });
  };
  reader.addEventListener("load", once, { once: true });

  const controller = new AbortController();
  reader.addEventListener(
    "load",
    () => __fileReaderCallbackCalls.push("aborted"),
    { signal: controller.signal }
  );
  controller.abort();

  reader.readAsText(new Blob(["first"]));
})()
"#,
    )
    .expect("first FileReader callback-interface read should start");

    vm.eval("__fileReaderCallbackTarget.readAsText(new Blob(['second']));")
        .expect("second FileReader callback-interface read should start");

    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
  calls: __fileReaderCallbackCalls,
  operationGets: __fileReaderOperationGets,
  callableOperationGets: __fileReaderCallableOperationGets,
  onceCalls: __fileReaderOnceCalls,
  result: __fileReaderCallbackTarget.result
})"#,
        )
        .expect("FileReader callback-interface facts should evaluate"),
        r#"{"calls":["object:true:true:true","callable:true:true","mutator","object:true:true:true","callable:true:true","mutator","late"],"operationGets":2,"callableOperationGets":0,"onceCalls":2,"result":"second"}"#
    );
}
#[test]
fn file_reader_listener_uses_callback_realm_and_exact_window_lifetime() {
    let mut vm = new_parsed_test_vm(
        "https://file-reader-callback-realm.test/",
        "<!doctype html><html><body></body></html>",
    );

    vm.eval(
        r#"
(() => {
  const iframe = document.createElement("iframe");
  iframe.srcdoc = "<!doctype html><html><body></body></html>";
  document.body.appendChild(iframe);
  globalThis.__fileReaderCallbackRealmFrame = iframe;
})()
"#,
    )
    .expect("cross-Realm FileReader listener setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    vm.eval(
        r#"
(() => {
  const other = __fileReaderCallbackRealmFrame.contentWindow;
  const reader = new FileReader();
  globalThis.__fileReaderRealmTarget = reader;
  globalThis.__fileReaderExpectedRealm = other;
  globalThis.__fileReaderRealmFacts = [];
  const callback = other.Function(
    "event",
    `"use strict";
     parent.__fileReaderRealmFacts.push([
       this === parent.__fileReaderRealmTarget,
       globalThis === parent.__fileReaderExpectedRealm,
       window.event === event,
       event.currentTarget === parent.__fileReaderRealmTarget
     ]);`
  );
  reader.addEventListener("load", callback);
  reader.readAsText(new Blob(["before-retirement"]));
})()
"#,
    )
    .expect("cross-Realm FileReader listener should register");

    vm.eval(
        r#"
__fileReaderCallbackRealmFrame.remove();
__fileReaderRealmTarget.readAsText(new Blob(["after-retirement"]));
"#,
    )
    .expect("FileReader should remain reusable after callback Window retirement");

    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
  facts: __fileReaderRealmFacts,
  childDetached: __fileReaderCallbackRealmFrame.contentWindow === null,
  result: __fileReaderRealmTarget.result
})"#,
        )
        .expect("cross-Realm FileReader listener facts should evaluate"),
        r#"{"facts":[[true,true,true,true]],"childDetached":true,"result":"after-retirement"}"#
    );
}
#[test]
fn file_reader_sync_is_not_exposed_on_window() {
    let mut vm = new_storage_test_vm("https://filereadersync-declared-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  return JSON.stringify({
    type: typeof FileReaderSync,
    own: Object.prototype.hasOwnProperty.call(globalThis, "FileReaderSync"),
    blobType: typeof Blob
  });
})()
"#,
        )
        .expect("FileReaderSync Window exposure probe should evaluate");

    assert_eq!(
        result,
        r#"{"type":"undefined","own":false,"blobType":"function"}"#
    );
}
#[test]
fn blob_parts_platform_detection_ignores_object_to_string_spoofing() {
    let mut vm = new_storage_test_vm("https://blob-platform-spoofing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const original = Object.prototype.toString;
  const doc = new DOMParser().parseFromString("<body><p></p></body>", "text/html");
  const list = doc.querySelectorAll("missing");
  let spoofedPlainObjectThrows = false;
  let realListSize = null;
  try {
    Object.prototype.toString = () => "[object NodeList]";
    try {
      new Blob({ 0: "x", length: 1 });
    } catch (error) {
      spoofedPlainObjectThrows = error instanceof TypeError;
    }
    Object.prototype.toString = () => { throw new Error("patched"); };
    realListSize = new Blob(list).size;
  } finally {
    Object.prototype.toString = original;
  }
  return JSON.stringify({ spoofedPlainObjectThrows, realListSize });
})()
"#,
        )
        .expect("Blob platform object detection probe should evaluate");

    assert_eq!(
        result,
        r#"{"spoofedPlainObjectThrows":true,"realListSize":0}"#
    );
}
#[test]
fn blob_internal_id_is_not_page_visible_or_forgeable() {
    let mut vm = new_storage_test_vm("https://blob-private-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const real = new Blob(["real"], { type: "text/plain" });
  const visibleOwnNames = Object.getOwnPropertyNames(real);
  const forged = { __lmBlobId: real.__lmBlobId ?? 1n };
  const probe = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  };
  return JSON.stringify({
    hasVisibleSlot: "__lmBlobId" in real,
    visibleOwnNames,
    objectUrl: probe(() => URL.createObjectURL(forged))
  });
})()
"#,
        )
        .expect("Blob private brand probe should evaluate");

    assert_eq!(
        result,
        r#"{"hasVisibleSlot":false,"visibleOwnNames":[],"objectUrl":"TypeError"}"#
    );
}
#[test]
fn blob_internal_builders_survive_global_blob_override() {
    let mut vm = new_storage_test_vm("https://blob-prototype-slot.test/");

    vm.eval(
        r#"
(() => {
  const blob = new Blob(["abcdef"], { type: "text/plain" });
  globalThis.Blob = function FakeBlob() {};
  Blob.prototype = {};
  const sliced = blob.slice(1, 4, "Text/Custom");
  globalThis.__blobSliceProbe = {
    type: sliced.type,
    ownNames: Object.getOwnPropertyNames(sliced),
    protoIsOriginal: Object.getPrototypeOf(sliced) === blob.__proto__
  };
  sliced.text().then(
    text => { globalThis.__blobSliceProbe.text = text; },
    error => { globalThis.__blobSliceProbe.error = error && error.name; }
  );
  return "scheduled";
})()
"#,
    )
    .expect("Blob global override probe should evaluate");

    vm.eval("0")
        .expect("Blob global override promise microtasks should drain");

    let result = vm
        .eval("JSON.stringify(globalThis.__blobSliceProbe)")
        .expect("Blob global override result should evaluate");

    assert_eq!(
        result,
        r#"{"type":"text/custom","ownNames":[],"protoIsOriginal":true,"text":"bcd"}"#
    );
}
#[test]
fn blob_slice_uses_receiver_realm_after_method_realm_is_detached() {
    let mut vm = new_parsed_test_vm(
        "https://blob-slice-receiver-realm.test/",
        "<!doctype html><html><body></body></html>",
    );

    vm.eval(
        r#"
(() => {
  const iframe = document.createElement("iframe");
  iframe.srcdoc = "<!doctype html><html><body></body></html>";
  document.body.appendChild(iframe);
  globalThis.__blobSliceRealmFrame = iframe;
})()
"#,
    )
    .expect("detached-Realm Blob slice setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    vm.eval(
        r#"
(() => {
  const iframe = __blobSliceRealmFrame;
  const detachedSlice = iframe.contentWindow.Blob.prototype.slice;
  iframe.remove();

  const blobSlice = detachedSlice.call(new Blob(["abcdef"]), 1, 4, "Text/Custom");
  const fileSlice = detachedSlice.call(new File(["uvwxyz"], "sample.txt"), 2, 5);
  globalThis.__blobSliceRealmBlobResult = blobSlice;
  globalThis.__blobSliceRealmFileResult = fileSlice;
  globalThis.__blobSliceRealmProbe = {
    childDetached: iframe.contentWindow === null,
    blobIsMainRealmBlob: blobSlice instanceof Blob,
    blobPrototypeIsMainRealmBlob: Object.getPrototypeOf(blobSlice) === Blob.prototype,
    fileSliceIsMainRealmBlob: fileSlice instanceof Blob,
    fileSliceIsFile: fileSlice instanceof File,
    fileSlicePrototypeIsMainRealmBlob: Object.getPrototypeOf(fileSlice) === Blob.prototype,
    type: blobSlice.type
  };
  Promise.all([blobSlice.text(), fileSlice.text()]).then(
    ([blobText, fileText]) => {
      __blobSliceRealmProbe.blobText = blobText;
      __blobSliceRealmProbe.fileText = fileText;
    },
    error => { __blobSliceRealmProbe.error = error && error.name; }
  );
  return "scheduled";
})()
"#,
    )
    .expect("detached-Realm Blob slice probe should evaluate");

    vm.eval("0")
        .expect("detached-Realm Blob slice promise microtasks should drain");

    let result = vm
        .eval("JSON.stringify(globalThis.__blobSliceRealmProbe)")
        .expect("detached-Realm Blob slice result should evaluate");

    assert_eq!(
        result,
        r#"{"childDetached":true,"blobIsMainRealmBlob":true,"blobPrototypeIsMainRealmBlob":true,"fileSliceIsMainRealmBlob":true,"fileSliceIsFile":false,"fileSlicePrototypeIsMainRealmBlob":true,"type":"text/custom","blobText":"bcd","fileText":"wxy"}"#
    );

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
        let context = scope.get_current_context();
        let global = context.global(scope);
        for name in ["__blobSliceRealmBlobResult", "__blobSliceRealmFileResult"] {
            let key = v8::String::new(scope, name).expect("result property name should allocate");
            let result = global
                .get(scope, key.into())
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .expect("slice result should be an object");
            assert_eq!(result.get_creation_context(scope), Some(context));
        }
        Ok(())
    })
    .expect("slice results should be allocated in the receiver's context");
}
#[test]
fn blob_slice_allocates_in_foreign_receiver_realm() {
    let mut vm = new_parsed_test_vm(
        "https://blob-slice-foreign-receiver.test/",
        "<!doctype html><html><body></body></html>",
    );

    vm.eval(
        r#"
(() => {
  const iframe = document.createElement("iframe");
  iframe.srcdoc = "<!doctype html><html><body></body></html>";
  document.body.appendChild(iframe);
  globalThis.__blobSliceForeignFrame = iframe;
})()
"#,
    )
    .expect("foreign-realm Blob slice setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const iframe = __blobSliceForeignFrame;
  const childBlob = iframe.contentWindow.Blob;
  const receiver = new childBlob(["abcdef"]);
  const sliced = Blob.prototype.slice.call(receiver, 1, 4);
  iframe.remove();
  const detachedSlice = Blob.prototype.slice.call(receiver, 2, 5);
  globalThis.__blobSliceForeignReceiver = receiver;
  globalThis.__blobSliceForeignResult = sliced;
  globalThis.__blobSliceForeignDetachedResult = detachedSlice;
  return JSON.stringify({
    prototypeIsChild: Object.getPrototypeOf(sliced) === childBlob.prototype,
    prototypeIsParent: Object.getPrototypeOf(sliced) === Blob.prototype,
    size: sliced.size,
    type: sliced.type,
    childDetached: iframe.contentWindow === null,
    detachedPrototypeIsChild: Object.getPrototypeOf(detachedSlice) === childBlob.prototype,
    detachedSize: detachedSlice.size
  });
})()
"#,
        )
        .expect("foreign-realm Blob slice should evaluate");
    assert_eq!(
        result,
        r#"{"prototypeIsChild":true,"prototypeIsParent":false,"size":3,"type":"","childDetached":true,"detachedPrototypeIsChild":true,"detachedSize":3}"#
    );

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
        let context = scope.get_current_context();
        let global = context.global(scope);
        let receiver_key = v8::String::new(scope, "__blobSliceForeignReceiver")
            .expect("receiver property name should allocate");
        let receiver = global
            .get(scope, receiver_key.into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .expect("receiver should be an object");
        let result_key = v8::String::new(scope, "__blobSliceForeignResult")
            .expect("result property name should allocate");
        let sliced = global
            .get(scope, result_key.into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .expect("slice result should be an object");
        let detached_result_key = v8::String::new(scope, "__blobSliceForeignDetachedResult")
            .expect("detached result property name should allocate");
        let detached_slice = global
            .get(scope, detached_result_key.into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .expect("detached slice result should be an object");
        let receiver_context = receiver
            .get_creation_context(scope)
            .expect("receiver should have a creation context");
        assert_ne!(receiver_context, context);
        assert_eq!(sliced.get_creation_context(scope), Some(receiver_context));
        assert_eq!(
            detached_slice.get_creation_context(scope),
            Some(receiver_context)
        );
        Ok(())
    })
    .expect("slice result should be allocated in the foreign receiver's context");
}
#[test]
fn blob_stream_is_native_readable_stream_and_response_consumes_bytes() {
    let mut vm = new_storage_test_vm("https://blob-stream-reader.test/");

    vm.eval(
        r#"
(() => {
  const blob = new Blob(["hello ", "世界", new Uint8Array([33])]);
  const stream = blob.stream();
  const reader = stream.getReader();
  globalThis.__blobStreamProbe = {
    streamIsReadable: stream instanceof ReadableStream,
    streamPrototype: Object.getPrototypeOf(stream) === ReadableStream.prototype,
    streamKeys: Object.keys(stream),
    streamOwnNames: Object.getOwnPropertyNames(stream),
    readerIsDefault: reader instanceof ReadableStreamDefaultReader,
    readerPrototype:
      Object.getPrototypeOf(reader) === ReadableStreamDefaultReader.prototype
  };
  reader.read()
    .then(first => {
      globalThis.__blobStreamProbe.firstDone = first.done;
      globalThis.__blobStreamProbe.firstValue = new TextDecoder().decode(first.value);
      return reader.read();
    })
    .then(second => {
      globalThis.__blobStreamProbe.secondDone = second.done;
      return new Response(blob.stream()).text();
    })
    .then(text => {
      globalThis.__blobStreamProbe.responseText = text;
    }, error => {
      globalThis.__blobStreamProbe.error = error && error.name;
    });
  return "scheduled";
})()
"#,
    )
    .expect("Blob stream reader probe should schedule");

    vm.eval("0")
        .expect("Blob stream reader promise chain should drain");

    let result = vm
        .eval("JSON.stringify(globalThis.__blobStreamProbe)")
        .expect("Blob stream reader result should evaluate");

    assert_eq!(
        result,
        r#"{"streamIsReadable":true,"streamPrototype":true,"streamKeys":[],"streamOwnNames":[],"readerIsDefault":true,"readerPrototype":true,"firstDone":false,"firstValue":"hello 世界!","secondDone":true,"responseText":"hello 世界!"}"#
    );
}
#[test]
fn zhihu_probe_navigator_profile_is_chromium_like() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            JSON.stringify({
              userAgent: navigator.userAgent,
              appVersion: navigator.appVersion,
              appCodeName: navigator.appCodeName,
              vendor: navigator.vendor,
              vendorSub: navigator.vendorSub,
              product: navigator.product,
              productSub: navigator.productSub
            })
            "#,
        )
        .expect("navigator profile probe should evaluate");

    let expected = format!(
        r#"{{"userAgent":"{}","appVersion":"{}","appCodeName":"Mozilla","vendor":"Google Inc.","vendorSub":"","product":"Gecko","productSub":"20030107"}}"#,
        DEFAULT_USER_AGENT,
        navigator_app_version(DEFAULT_USER_AGENT)
    );
    assert_eq!(result, expected);
}
#[test]
fn opfs_root_directory_and_file_handles_share_partition_service() {
    let mut vm = new_storage_page_task_executor_test_vm("https://opfs-basic.test/");

    vm.exec(
        r#"
        globalThis.__opfsBasicProbe = "pending";
        (async () => {
          const getPrototypeOf = Reflect.getPrototypeOf;
          const asyncIteratorPrototype = getPrototypeOf(
            getPrototypeOf(async function*() {}).prototype
          );
          const originalObjectGetPrototypeOf = Object.getPrototypeOf;
          const root = await navigator.storage.getDirectory();
          const empty = await root.getFileHandle("empty.txt", { create: true });
          const snapshot = await empty.getFile();
          const directory = await root.getDirectoryHandle("dir", { create: true });
          const nested = await directory.getFileHandle("nested", { create: true });
          Object.getPrototypeOf = function poisonedGetPrototypeOf() {
            throw new Error("public Object.getPrototypeOf was observed");
          };
          let iteratorShape;
          try {
            const iterator = root.values();
            const prototype = getPrototypeOf(iterator);
            const next = Object.getOwnPropertyDescriptor(prototype, "next");
            const tag = Object.getOwnPropertyDescriptor(prototype, Symbol.toStringTag);
            iteratorShape = [
              getPrototypeOf(prototype) === asyncIteratorPrototype,
              iterator[Symbol.asyncIterator]() === iterator,
              !Object.hasOwn(iterator, "next"),
              !Object.hasOwn(iterator, Symbol.asyncIterator),
              !Object.hasOwn(prototype, "constructor"),
              next?.enumerable === true &&
                next?.writable === true &&
                next?.configurable === true &&
                next?.value?.length === 0,
              tag?.value === "FileSystemDirectoryHandle AsyncIterator" &&
                tag?.enumerable === false &&
                tag?.writable === false &&
                tag?.configurable === true
            ];
          } finally {
            Object.getPrototypeOf = originalObjectGetPrototypeOf;
          }
          const keys = [];
          for await (const key of root.keys()) keys.push(key);
          keys.sort();
          const entries = [];
          for await (const [name, handle] of root) {
            entries.push(`${name}:${handle.kind}:${handle instanceof FileSystemHandle}`);
          }
          entries.sort();

          const bucket = await navigator.storageBuckets.open("default");
          const bucketRoot = await bucket.getDirectory();
          await bucketRoot.getFileHandle("bucket-only", { create: true });
          const defaultKeys = [];
          for await (const key of root.keys()) defaultKeys.push(key);
          const bucketKeys = [];
          for await (const key of bucketRoot.keys()) bucketKeys.push(key);

          const resolved = await root.resolve(nested);

          await directory.remove({ recursive: true });
          const afterRemove = [];
          for await (const key of root.keys()) afterRemove.push(key);

          return {
            managerMethod: typeof StorageManager.prototype.getDirectory,
            syncAccessHandleConstructor: typeof FileSystemSyncAccessHandle,
            syncAccessHandleMethod:
              typeof FileSystemFileHandle.prototype.createSyncAccessHandle,
            root: [root.kind, root.name, root instanceof FileSystemDirectoryHandle,
                   root instanceof FileSystemHandle],
            file: [empty.kind, empty.name, snapshot.name, snapshot.size,
                   snapshot instanceof File],
            same: await empty.isSameEntry(await root.getFileHandle("empty.txt")),
            iteratorShape,
            resolved,
            keys,
            entries,
            defaultKeys: defaultKeys.sort(),
            bucketKeys: bucketKeys.sort(),
            afterRemove: afterRemove.sort()
          };
        })().then(
          value => { globalThis.__opfsBasicProbe = JSON.stringify(value); },
          error => { globalThis.__opfsBasicProbe = `error:${error && error.name}:${error && error.message}`; }
        );
        "#,
        None,
    )
    .expect("OPFS basic probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__opfsBasicProbe)")
        .expect("OPFS basic probe should settle");

    assert_eq!(
        result,
        r#"{"managerMethod":"function","syncAccessHandleConstructor":"undefined","syncAccessHandleMethod":"undefined","root":["directory","",true,true],"file":["file","empty.txt","empty.txt",0,true],"same":true,"iteratorShape":[true,true,true,true,true,true,true],"resolved":["dir","nested"],"keys":["dir","empty.txt"],"entries":["dir:directory:true","empty.txt:file:true"],"defaultKeys":["dir","empty.txt"],"bucketKeys":["bucket-only"],"afterRemove":["empty.txt"]}"#
    );
}
#[tokio::test]
async fn network_dedicated_worker_file_snapshot_clone_retains_opfs_incarnation() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/opfs-snapshot-clone-worker.js",
        "text/javascript; charset=utf-8",
        r#"
        let snapshot;
        self.onmessage = async event => {
          try {
            if (event.data.type === "init") {
              snapshot = event.data.snapshot;
              postMessage({ type: "ready" });
              return;
            }
            const root = await navigator.storage.getDirectory();
            const target = await root.getFileHandle("worker-target.txt", { create: true });
            const writer = await target.createWritable();
            let writeResult;
            try {
              await writer.write(snapshot);
              await writer.close();
              writeResult = "resolved";
            } catch (error) {
              writeResult = error && error.name || typeof error;
            }
            postMessage({
              type: "result",
              brand: snapshot instanceof File,
              text: await snapshot.text(),
              writeResult,
              targetSize: (await target.getFile()).size
            });
          } catch (error) {
            postMessage({ type: "result", error: `${error && error.name}:${error && error.message}` });
          }
        };
        "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        globalThis.__opfsWorkerSnapshotCloneProbe = "pending";
        (async () => {
          const root = await navigator.storage.getDirectory();
          const source = await root.getFileHandle("worker-source.txt", { create: true });
          const initialWriter = await source.createWritable();
          await initialWriter.write("worker snapshot");
          await initialWriter.close();
          const snapshot = await source.getFile();
          const worker = new Worker("opfs-snapshot-clone-worker.js");
          worker.onmessage = async event => {
            try {
              if (event.data.type === "ready") {
                await root.removeEntry("worker-source.txt");
                const replacement = await root.getFileHandle(
                  "worker-source.txt",
                  { create: true }
                );
                const replacementWriter = await replacement.createWritable();
                await replacementWriter.write("worker snapshot");
                await replacementWriter.close();
                worker.postMessage({ type: "validate" });
                return;
              }
              globalThis.__opfsWorkerSnapshotCloneProbe = JSON.stringify(event.data);
            } catch (error) {
              globalThis.__opfsWorkerSnapshotCloneProbe =
                `error:${error && error.name}:${error && error.message}`;
            }
          };
          worker.onmessageerror = () => {
            globalThis.__opfsWorkerSnapshotCloneProbe = "messageerror";
          };
          worker.onerror = event => {
            globalThis.__opfsWorkerSnapshotCloneProbe = `error:${event.message}`;
          };
          worker.postMessage({ type: "init", snapshot });
        })().catch(error => {
          globalThis.__opfsWorkerSnapshotCloneProbe =
            `error:${error && error.name}:${error && error.message}`;
        });
        "#,
    )
    .expect("network Worker OPFS snapshot clone probe should schedule");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__opfsWorkerSnapshotCloneProbe)",
        r#"{"type":"result","brand":true,"text":"worker snapshot","writeResult":"NotFoundError","targetSize":0}"#,
    )
    .await;

    server
        .await
        .expect("network Worker OPFS snapshot clone script server should finish");
}
#[test]
fn opfs_file_move_updates_handle_and_enforces_locks_bucket_identity_and_quota() {
    let mut vm = new_storage_page_task_executor_test_vm("https://opfs-file-move.test/");

    vm.exec(
        r#"
        globalThis.__opfsFileMoveProbe = "pending";
        (async () => {
          const outcome = async promise => {
            try {
              await promise;
              return "resolved";
            } catch (error) {
              return error && error.name || typeof error;
            }
          };
          const text = async handle => new Response(await handle.getFile()).text();
          const root = await navigator.storage.getDirectory();
          const source = await root.getDirectoryHandle("source", { create: true });
          const destination = await root.getDirectoryHandle("destination", { create: true });
          const file = await source.getFileHandle("before.txt", { create: true });
          const writer = await file.createWritable();
          await writer.write("moved bytes");
          await writer.close();

          await file.move(destination, "moved.txt");
          const destinationHandle = await destination.getFileHandle("moved.txt");
          const sameAfterFirstMove = await file.isSameEntry(destinationHandle);
          const active = await file.createWritable({ keepExistingData: true });
          const sourceLock = await outcome(file.move("blocked.txt"));
          await active.close();
          await file.move("renamed.txt");

          const otherBucket = await navigator.storageBuckets.open("other");
          const otherRoot = await otherBucket.getDirectory();
          const crossBucket = await outcome(file.move(otherRoot));

          const quotaBucket = await navigator.storageBuckets.open("quota", { quota: 150 });
          const quotaRoot = await quotaBucket.getDirectory();
          const quotaFile = await quotaRoot.getFileHandle("a", { create: true });
          const quotaRename = await outcome(quotaFile.move("a-much-longer-name"));
          const quotaKeys = [];
          for await (const key of quotaRoot.keys()) quotaKeys.push(key);

          return {
            prototype: [
              typeof FileSystemFileHandle.prototype.move,
              Object.prototype.hasOwnProperty.call(FileSystemFileHandle.prototype, "move"),
              Object.prototype.hasOwnProperty.call(FileSystemDirectoryHandle.prototype, "move")
            ],
            moved: [
              file.name,
              await text(file),
              sameAfterFirstMove,
              await root.resolve(file),
              await outcome(source.getFileHandle("before.txt")),
              await outcome(destination.getFileHandle("moved.txt"))
            ],
            sourceLock,
            crossBucket,
            quotaRename,
            quotaFileName: quotaFile.name,
            quotaKeys
          };
        })().then(
          value => { globalThis.__opfsFileMoveProbe = JSON.stringify(value); },
          error => {
            globalThis.__opfsFileMoveProbe =
              `error:${error && error.name}:${error && error.message}`;
          }
        );
        "#,
        None,
    )
    .expect("OPFS file move probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__opfsFileMoveProbe)")
        .expect("OPFS file move probe should settle");

    assert_eq!(
        result,
        r#"{"prototype":["function",true,true],"moved":["renamed.txt","moved bytes",true,["destination","renamed.txt"],"NotFoundError","NotFoundError"],"sourceLock":"NoModificationAllowedError","crossBucket":"InvalidModificationError","quotaRename":"QuotaExceededError","quotaFileName":"a","quotaKeys":["a"]}"#
    );
}
#[test]
fn opfs_file_snapshot_identity_survives_runtime_structured_clone() {
    let mut vm = new_storage_page_task_executor_test_vm("https://opfs-snapshot-identity.test/");

    vm.exec(
        r#"
        globalThis.__opfsSnapshotIdentityProbe = "pending";
        (async () => {
          const root = await navigator.storage.getDirectory();
          const source = await root.getFileHandle("source.txt", { create: true });
          const replace = async bytes => {
            const writer = await source.createWritable();
            await writer.write(bytes);
            await writer.close();
          };
          const writeSnapshot = async (name, snapshot) => {
            const target = await root.getFileHandle(name, { create: true });
            const writer = await target.createWritable();
            let result;
            try {
              await writer.write(snapshot);
              await writer.close();
              result = "resolved";
            } catch (error) {
              result = error && error.name || typeof error;
            }
            return [result, (await target.getFile()).size];
          };

          await replace("version-one");
          const first = await source.getFile();
          const firstClone = structuredClone(first);
          const validClone = await writeSnapshot("valid-clone.txt", firstClone);

          await replace("version-two");
          const modifiedDirect = await writeSnapshot("modified-direct.txt", first);
          const modifiedClone = await writeSnapshot("modified-clone.txt", firstClone);

          const second = await source.getFile();
          const secondClone = structuredClone(second);
          await root.removeEntry("source.txt");
          const replacement = await root.getFileHandle("source.txt", { create: true });
          const replacementWriter = await replacement.createWritable();
          await replacementWriter.write("version-two");
          await replacementWriter.close();
          const recreatedDirect = await writeSnapshot("recreated-direct.txt", second);
          const recreatedClone = await writeSnapshot("recreated-clone.txt", secondClone);

          return {
            validClone,
            validText: await (await root.getFileHandle("valid-clone.txt")).getFile()
              .then(file => file.text()),
            modifiedDirect,
            modifiedClone,
            recreatedDirect,
            recreatedClone,
            cloneBrand: firstClone instanceof File,
            cloneMetadataHidden: Object.getOwnPropertyNames(firstClone)
              .every(name => !name.startsWith("__moli"))
          };
        })().then(
          value => { globalThis.__opfsSnapshotIdentityProbe = JSON.stringify(value); },
          error => {
            globalThis.__opfsSnapshotIdentityProbe =
              `error:${error && error.name}:${error && error.message}`;
          }
        );
        "#,
        None,
    )
    .expect("OPFS snapshot identity probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__opfsSnapshotIdentityProbe)")
        .expect("OPFS snapshot identity probe should settle");

    assert_eq!(
        result,
        r#"{"validClone":["resolved",11],"validText":"version-one","modifiedDirect":["NotFoundError",0],"modifiedClone":["NotFoundError",0],"recreatedDirect":["NotFoundError",0],"recreatedClone":["NotFoundError",0],"cloneBrand":true,"cloneMetadataHidden":true}"#
    );
}
#[test]
fn blob_url_revocation_respects_browser_partitions_and_allows_same_origin_realms() {
    for document_url in ["https://blob-url-revocation.test/", "data:text/html,opaque"] {
        let markup = "<!doctype html><html><body></body></html>";
        let mut creator = new_parsed_test_vm(document_url, markup);
        let mut other_partition = new_parsed_test_vm(document_url, markup);
        let url = creator
            .eval("URL.createObjectURL(new Blob(['payload']))")
            .expect("create object URL");
        let url_literal = serde_json::to_string(&url).unwrap();
        other_partition
            .eval(&format!("URL.revokeObjectURL({url_literal})"))
            .expect("foreign partition revocation is a silent no-op");
        assert_eq!(
            crate::blob::object_url_body_and_type(&url).unwrap().0,
            "payload"
        );
        // The initial about:blank child inherits even its opaque parent's key.
        creator
            .eval(&format!(
                r#"(() => {{
                    const frame = document.createElement('iframe');
                    document.body.appendChild(frame);
                    const revoke = frame.contentWindow.URL.revokeObjectURL;
                    revoke.call(null, {url_literal});
                }})()"#
            ))
            .expect("same-origin child can revoke parent URL");
        assert!(crate::blob::object_url_body_and_type(&url).is_none());
    }
}

#[test]
fn removing_child_frame_revokes_only_its_blob_object_urls() {
    let mut vm = new_parsed_test_vm(
        "https://blob-url-child-lifetime.test/",
        "<!doctype html><html><body></body></html>",
    );

    let setup = r#"
(() => {
  const parentUrl = URL.createObjectURL(new Blob(["parent"]));
  const frame = document.createElement("iframe");
  document.body.appendChild(frame);
  const childUrl = frame.contentWindow.URL.createObjectURL(
    new frame.contentWindow.Blob(["child"])
  );
  globalThis.__blobUrlLifetimeFrame = frame;
  return `${parentUrl}|${childUrl}`;
})()
"#;
    let urls = vm
        .eval(setup)
        .expect("child Blob object URL setup should evaluate");
    let (parent_url, child_url) = urls
        .split_once('|')
        .expect("setup should return both object URLs");

    // A separate runtime starts its realm counter at the same value. Its
    // object URLs must survive retirement of the first runtime's child.
    let mut other_vm = new_parsed_test_vm(
        "https://blob-url-child-lifetime.test/other",
        "<!doctype html><html><body></body></html>",
    );
    let other_urls = other_vm.eval(setup).expect("other runtime's object URLs");
    let (_, other_child_url) = other_urls.split_once('|').expect("other child URL");

    assert_eq!(
        crate::blob::object_url_body_and_type(parent_url),
        Some(("parent".to_owned(), String::new()))
    );
    assert_eq!(
        crate::blob::object_url_body_and_type(child_url),
        Some(("child".to_owned(), String::new()))
    );

    vm.eval("globalThis.__blobUrlLifetimeFrame.remove()")
        .expect("child frame removal should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    assert_eq!(
        crate::blob::object_url_body_and_type(parent_url),
        Some(("parent".to_owned(), String::new())),
        "removing a child frame must preserve the parent realm's object URLs"
    );
    assert!(
        crate::blob::object_url_body_and_type(child_url).is_none(),
        "removing a child frame must revoke object URLs created by its realm"
    );
    assert_eq!(
        crate::blob::object_url_body_and_type(other_child_url),
        Some(("child".to_owned(), String::new())),
        "realm token reuse in another runtime must not revoke that runtime's URLs"
    );
}

#[test]
fn blob_members_enforce_webidl_receiver_contracts() {
    let mut vm = new_storage_test_vm("https://blob-receiver-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  const rejectionName = callback => {
    try {
      return Promise.resolve(callback()).then(
        () => "resolved",
        error => error && error.name,
      );
    } catch (error) {
      return Promise.resolve("threw:" + (error && error.name));
    }
  };
  const sizeGetter = Object.getOwnPropertyDescriptor(Blob.prototype, "size").get;
  const typeGetter = Object.getOwnPropertyDescriptor(Blob.prototype, "type").get;
  globalThis.__blobReceiverProbe = {
    syncThrows: [
      throwsTypeError(() => sizeGetter.call({})),
      throwsTypeError(() => typeGetter.call({})),
      throwsTypeError(() => Blob.prototype.slice.call(null)),
      throwsTypeError(() => Blob.prototype.stream.call(null)),
    ],
  };
  Promise.all([
    rejectionName(() => Blob.prototype.text.call(null)),
    rejectionName(() => Blob.prototype.arrayBuffer.call(null)),
    rejectionName(() => Blob.prototype.bytes.call(null)),
  ]).then(names => {
    __blobReceiverProbe.promiseRejections = names;
  });
  return "scheduled";
})()
"#,
        )
        .expect("Blob receiver contract probe should evaluate");

    assert_eq!(result, "scheduled");
    vm.eval("0")
        .expect("Blob receiver rejection microtasks should drain");

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__blobReceiverProbe)")
            .expect("Blob receiver contract result should evaluate"),
        r#"{"syncThrows":[true,true,true,true],"promiseRejections":["TypeError","TypeError","TypeError"]}"#
    );
}
