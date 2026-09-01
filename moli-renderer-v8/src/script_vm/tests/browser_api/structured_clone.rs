use super::*;

#[test]
fn structured_clone_deserializes_in_the_receiver_realm() {
    let mut vm = new_storage_test_vm("https://structured-clone-receiver-realm.test/");

    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          (document.body || document.documentElement || document).appendChild(frame);
          globalThis.__structuredCloneRealmFrame = frame;
          return "ready";
        })()
        "#,
    )
    .expect("structuredClone receiver Realm setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = globalThis.__structuredCloneRealmFrame;
              const child = frame.contentWindow;
              const constructors = ["Object", "Array", "Date", "RegExp"];
              const intoTop = constructors.map(name => {
                const clone = child.structuredClone.call(window, new child[name]);
                return Object.getPrototypeOf(clone) === window[name].prototype;
              });
              const intoChild = constructors.map(name => {
                const clone = window.structuredClone.call(child, new window[name]);
                return Object.getPrototypeOf(clone) === child[name].prototype;
              });

              const buffer = new child.ArrayBuffer(8);
              const transferred = child.structuredClone.call(window, buffer, {
                transfer: [buffer]
              });

              const exceptionRealm = callback => {
                try {
                  callback();
                  return "missing";
                } catch (error) {
                  return error instanceof child.TypeError ? "child" : "other";
                }
              };
              const illegalInvocation = exceptionRealm(() =>
                child.structuredClone.call({}, 1));
              const missingArgument = exceptionRealm(() =>
                child.structuredClone.call(window));

              frame.remove();
              const detached = child.structuredClone("detached") === undefined;

              return JSON.stringify({
                intoTop,
                intoChild,
                transfer: [
                  buffer.byteLength,
                  transferred.byteLength,
                  transferred instanceof window.ArrayBuffer
                ],
                illegalInvocation,
                missingArgument,
                detached
              });
            })()
            "#,
        )
        .expect("cross-Realm structuredClone probe should evaluate");

    assert_eq!(
        result,
        r#"{"intoTop":[true,true,true,true],"intoChild":[true,true,true,true],"transfer":[0,8,true],"illegalInvocation":"child","missingArgument":"child","detached":true}"#,
    );
}

#[test]
fn structured_clone_preserves_webassembly_module() {
    let mut vm = new_storage_test_vm("https://example.com/wasm-clone");

    let result = vm
        .eval(
            r#"
            (() => {
              const module = new WebAssembly.Module(
                new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])
              );
              const clone = structuredClone(module);
              const instance = new WebAssembly.Instance(clone, {});
              return [
                clone instanceof WebAssembly.Module,
                clone === module,
                Object.keys(instance.exports).length
              ].join("|");
            })()
            "#,
        )
        .expect("WebAssembly.Module structuredClone should evaluate");

    assert_eq!(result, "true|false|0");
}

#[test]
fn structured_clone_preserves_file_list_brand_contents_and_graph_identity() {
    let mut vm = new_storage_test_vm("https://file-list-structured-clone.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const transfer = new DataTransfer();
              const file = new File(["file bytes"], "note.txt", {
                type: "text/custom",
                lastModified: 42
              });
              file.expando = "not serialized";
              transfer.items.add(file);
              const list = transfer.files;
              list.expando = "not serialized";

              let indexedGetterHits = 0;
              Object.defineProperty(list, "0", {
                configurable: true,
                get() {
                  indexedGetterHits++;
                  throw new Error("FileList serialization invoked an indexed getter");
                }
              });

              const listFirst = structuredClone({ list, alias: list, file });
              const fileFirst = structuredClone({ file, list });
              const empty = structuredClone(new DataTransfer().files);

              return JSON.stringify({
                listBrand: listFirst.list instanceof FileList,
                listPrototype: Object.getPrototypeOf(listFirst.list) === FileList.prototype,
                listDistinct: listFirst.list !== list,
                listAlias: listFirst.list === listFirst.alias,
                listLength: listFirst.list.length,
                itemMatchesFile: listFirst.list.item(0) === listFirst.file,
                indexedMatchesFile: listFirst.list[0] === listFirst.file,
                fileFirstIdentity: fileFirst.list[0] === fileFirst.file,
                fileBrand: listFirst.file instanceof File && listFirst.file instanceof Blob,
                fileDistinct: listFirst.file !== file,
                fileMetadata: [
                  listFirst.file.name,
                  listFirst.file.type,
                  listFirst.file.lastModified,
                  listFirst.file.size
                ],
                expandosExcluded:
                  listFirst.list.expando === undefined && listFirst.file.expando === undefined,
                indexedGetterHits,
                empty: [
                  empty instanceof FileList,
                  Object.getPrototypeOf(empty) === FileList.prototype,
                  empty.length,
                  empty.item(0) === null
                ]
              });
            })()
            "#,
        )
        .expect("FileList structuredClone probe should evaluate");

    assert_eq!(
        result,
        r#"{"listBrand":true,"listPrototype":true,"listDistinct":true,"listAlias":true,"listLength":1,"itemMatchesFile":true,"indexedMatchesFile":true,"fileFirstIdentity":true,"fileBrand":true,"fileDistinct":true,"fileMetadata":["note.txt","text/custom",42,10],"expandosExcluded":true,"indexedGetterHits":0,"empty":[true,true,0,true]}"#,
    );
}

#[test]
fn structured_clone_rejects_native_dom_nodes() {
    let mut vm = new_storage_test_vm("https://example.com/dom-node-clone");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = value => {
                try {
                  structuredClone(value);
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              };
              const liveElement = document.createElement("div");
              const detached = document.implementation.createHTMLDocument("detached");
              return [
                probe(document),
                probe(liveElement),
                probe(detached),
                probe(detached.body)
              ].join("|");
            })()
            "#,
        )
        .expect("DOM node structuredClone probe should evaluate");

    assert_eq!(
        result,
        "DataCloneError|DataCloneError|DataCloneError|DataCloneError"
    );
}

#[test]
fn structured_clone_rejects_real_performance_entries_without_prototype_spoofing() {
    let document_url = Url::parse("https://performance-entry-clone.test/").expect("document URL");
    let mut vm = new_storage_test_vm(document_url.as_str());
    vm.record_script_subresource_network_result(
        document_url.clone(),
        document_url.join("resource.js").expect("resource URL"),
        &Err("synthetic resource failure".to_owned()),
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = callback => {
                try {
                  callback();
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              };
              const navigation = performance.getEntriesByType("navigation")[0];
              const resource = performance.getEntriesByType("resource")[0];
              performance.mark("clone-mark");
              const mark = performance.getEntriesByName("clone-mark")[0];
              performance.measure("clone-measure");
              const measure = performance.getEntriesByName("clone-measure")[0];
              navigation.expando = "still not clonable";

              const fake = Object.create(PerformanceEntry.prototype);
              fake.value = 7;
              const fakeClone = structuredClone(fake);
              const plainClone = structuredClone({
                name: navigation.name,
                entryType: navigation.entryType,
                startTime: navigation.startTime,
                duration: navigation.duration
              });

              return JSON.stringify({
                navigation: probe(() => structuredClone(navigation)),
                resource: probe(() => structuredClone(resource)),
                mark: probe(() => structuredClone(mark)),
                measure: probe(() => structuredClone(measure)),
                postMessage: probe(() => window.postMessage(navigation, "*")),
                fake: [fakeClone.value, fakeClone instanceof PerformanceEntry],
                plain: [plainClone.entryType, typeof plainClone.startTime]
              });
            })()
            "#,
        )
        .expect("PerformanceEntry structured clone probe should evaluate");

    assert_eq!(
        result,
        r#"{"navigation":"DataCloneError","resource":"DataCloneError","mark":"DataCloneError","measure":"DataCloneError","postMessage":"DataCloneError","fake":[7,false],"plain":["navigation","number"]}"#
    );
}

#[test]
fn structured_clone_performance_entry_brand_is_private_and_survives_prototype_changes() {
    let mut vm = new_storage_test_vm("https://performance-entry-brand.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = callback => {
                try {
                  callback();
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              };
              const mark = performance.mark("factory-mark");
              const constructed = new PerformanceMark("constructed-mark");
              class DerivedMark extends PerformanceMark {}
              const derived = new DerivedMark("derived-mark");
              const inherited = Object.create(mark);
              inherited.value = 7;
              const inheritedClone = structuredClone(inherited);
              const forgedClone = structuredClone({ __moliWebApiType: true });
              const hiddenBrand = !Reflect.ownKeys(mark).includes("__moliWebApiType");
              mark.__moliWebApiType = false;
              Object.setPrototypeOf(mark, null);
              Object.setPrototypeOf(constructed, Object.prototype);

              return JSON.stringify({
                hiddenBrand,
                alteredFactory: probe(() => structuredClone(mark)),
                alteredConstructor: probe(() => structuredClone(constructed)),
                derived: probe(() => structuredClone(derived)),
                nested: probe(() => structuredClone({ entry: mark })),
                detail: probe(() => performance.mark("entry-as-detail", { detail: mark })),
                inherited: [inheritedClone.value, inheritedClone instanceof PerformanceEntry],
                forged: [forgedClone.__moliWebApiType, forgedClone instanceof PerformanceEntry]
              });
            })()
            "#,
        )
        .expect("PerformanceEntry private brand probe should evaluate");

    assert_eq!(
        result,
        r#"{"hiddenBrand":true,"alteredFactory":"DataCloneError","alteredConstructor":"DataCloneError","derived":"DataCloneError","nested":"DataCloneError","detail":"DataCloneError","inherited":[7,false],"forged":[true,false]}"#
    );
}

#[test]
fn structured_clone_preserves_dom_exception_fields_and_brand() {
    let mut vm = new_storage_test_vm("https://dom-exception-clone.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const source = new DOMException("chunk could not be cloned", "DataCloneError");
              source.expando = "not serialized";
              const clone = structuredClone(source);
              return [
                clone instanceof DOMException,
                Object.getPrototypeOf(clone) === DOMException.prototype,
                clone.name,
                clone.message,
                clone.code,
                clone.expando === undefined
              ].join("|");
            })()
            "#,
        )
        .expect("DOMException structured clone should evaluate");

    assert_eq!(
        result,
        "true|true|DataCloneError|chunk could not be cloned|25|true"
    );
}

#[test]
fn structured_clone_preserves_geometry_interfaces_and_internal_values() {
    let mut vm = new_storage_test_vm("https://geometry-structured-clone.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const same = (actual, expected) => Object.is(actual, expected);
              const check = (name, source, attrs, expected, nested = false) => {
                const originals = nested ? attrs.map(attr => source[attr]) : [];
                source.expando = "not serialized";
                for (const attr of attrs) {
                  Object.defineProperty(source, attr, {
                    get() { throw new Error(`unexpected ${name}.${attr} getter`); }
                  });
                }
                const clone = structuredClone(source);
                const valuesMatch = attrs.every((attr, index) => {
                  if (!nested) return same(clone[attr], expected[index]);
                  return clone[attr] !== originals[index]
                    && expected[index].every((value, component) =>
                      same(clone[attr][["x", "y", "z", "w"][component]], value));
                });
                return Object.prototype.toString.call(clone) === `[object ${name}]`
                  && Object.getPrototypeOf(clone) === self[name].prototype
                  && !("expando" in clone)
                  && valuesMatch;
              };

              const pointAttrs = ["x", "y", "z", "w"];
              const rectAttrs = ["x", "y", "width", "height"];
              const quadAttrs = ["p1", "p2", "p3", "p4"];
              const pointValues = [1, -0, Infinity, NaN];
              const quadValues = [
                [1, 2, 3, 4],
                [-0, -0, -0, -0],
                [Infinity, Infinity, Infinity, Infinity],
                [NaN, NaN, NaN, NaN]
              ];
              const matrixAttrs = [
                "a", "b", "c", "d", "e", "f",
                "m11", "m12", "m13", "m14",
                "m21", "m22", "m23", "m24",
                "m31", "m32", "m33", "m34",
                "m41", "m42", "m43", "m44", "is2D"
              ];
              const normalized2DAttrs = new Set([
                "m13", "m14", "m23", "m24",
                "m31", "m32", "m34", "m43"
              ]);
              const matrixCheck = (name, values) => {
                const source = new self[name](values);
                if (values.length === 6) {
                  for (const attr of normalized2DAttrs) source[attr] = -0;
                }
                const expected = matrixAttrs.map(attr =>
                  normalized2DAttrs.has(attr) && values.length === 6 ? 0 : source[attr]);
                return check(name, source, matrixAttrs, expected);
              };

              return [
                check("DOMPointReadOnly", new DOMPointReadOnly(...pointValues),
                  pointAttrs, pointValues),
                check("DOMPoint", new DOMPoint(...pointValues), pointAttrs, pointValues),
                check("DOMRectReadOnly", new DOMRectReadOnly(...pointValues),
                  rectAttrs, pointValues),
                check("DOMRect", new DOMRect(...pointValues), rectAttrs, pointValues),
                check("DOMQuad", new DOMQuad(...quadValues.map(
                  ([x, y, z, w]) => ({x, y, z, w}))), quadAttrs, quadValues, true),
                matrixCheck("DOMMatrixReadOnly", [1, -0, Infinity, NaN, 5, 6]),
                matrixCheck("DOMMatrix", [
                  11, -0, Infinity, NaN, 21, 22, 23, 24,
                  31, 32, 33, 34, 41, 42, 43, 44
                ])
              ].join("|");
            })()
            "#,
        )
        .expect("Geometry structuredClone probe should evaluate");

    assert_eq!(result, "true|true|true|true|true|true|true");
}

#[test]
fn structured_clone_transfers_array_buffer_and_preserves_view_aliases() {
    let mut vm = new_storage_test_vm("https://array-buffer-transfer.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const buffer = new ArrayBuffer(10);
              const data = new DataView(buffer);
              data.setUint32(0, 0x01020304, false);
              const words = new Uint16Array(buffer, 4, 3);
              words.set([7, 500, 65535]);
              const clone = structuredClone(
                { buffer, data, words, nested: new Map([["words", words]]) },
                { transfer: [buffer] }
              );
              let duplicate;
              const duplicateBuffer = new ArrayBuffer(1);
              try {
                structuredClone(duplicateBuffer, {
                  transfer: [duplicateBuffer, duplicateBuffer]
                });
                duplicate = "missing";
              } catch (error) {
                duplicate = error.name;
              }
              return JSON.stringify({
                sourceLength: buffer.byteLength,
                cloneLength: clone.buffer.byteLength,
                bytes: Array.from(new Uint8Array(clone.buffer)),
                aliases: [
                  clone.data.buffer === clone.buffer,
                  clone.words.buffer === clone.buffer,
                  clone.nested.get("words").buffer === clone.buffer
                ],
                brands: [
                  clone.data instanceof DataView,
                  clone.words instanceof Uint16Array,
                  clone.nested instanceof Map
                ],
                duplicate
              });
            })()
            "#,
        )
        .expect("ArrayBuffer structuredClone transfer should evaluate");

    assert_eq!(
        result,
        r#"{"sourceLength":0,"cloneLength":10,"bytes":[1,2,3,4,7,0,244,1,255,255],"aliases":[true,true,true],"brands":[true,true,true],"duplicate":"DataCloneError"}"#
    );
}

#[test]
fn structured_clone_preserves_opfs_locator_and_source_handle() {
    let mut vm = new_storage_page_task_executor_test_vm("https://opfs-structured-clone.test/");

    vm.exec(
        r#"
        globalThis.__opfsStructuredCloneProbe = "pending";
        (async () => {
          const root = await navigator.storage.getDirectory();
          const directory = await root.getDirectoryHandle("clone-dir", { create: true });
          const file = await directory.getFileHandle("clone.txt", { create: true });
          const writer = await file.createWritable();
          await writer.write("clone bytes");
          await writer.close();

          const cloned = structuredClone({ root, handles: [file, file] });
          const clonedFile = cloned.handles[0];
          globalThis.__opfsStructuredCloneProbe = JSON.stringify({
            rootBrand: cloned.root instanceof FileSystemDirectoryHandle,
            fileBrand: clonedFile instanceof FileSystemFileHandle,
            sharedReference: cloned.handles[0] === cloned.handles[1],
            distinctFromSource: clonedFile !== file,
            sameEntry: await clonedFile.isSameEntry(file),
            resolved: await cloned.root.resolve(clonedFile),
            text: await (await clonedFile.getFile()).text()
          });
        })().catch(error => {
          globalThis.__opfsStructuredCloneProbe =
            `error:${error && error.name}:${error && error.message}`;
        });
        "#,
        None,
    )
    .expect("OPFS structuredClone probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__opfsStructuredCloneProbe)")
        .expect("OPFS structuredClone probe should settle");
    assert_eq!(
        result,
        r#"{"rootBrand":true,"fileBrand":true,"sharedReference":true,"distinctFromSource":true,"sameEntry":true,"resolved":["clone-dir","clone.txt"],"text":"clone bytes"}"#
    );
}

#[tokio::test]
async fn blob_iframe_inherits_secure_origin_for_opfs_handle_messages() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://opfs-blob-frame-clone.test/page.html",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__opfsBlobFrameCloneProbe = "pending";
  let frame;
  addEventListener("message", async event => {
    if (!event.data || event.source !== frame.contentWindow) {
      return;
    }
    if (event.data.kind === "ready") {
      const root = await navigator.storage.getDirectory();
      frame.contentWindow.postMessage({ kind: "handle", root }, "*");
      return;
    }
    if (event.data.kind === "result" || event.data.kind === "messageerror") {
      globalThis.__opfsBlobFrameCloneProbe = JSON.stringify(event.data);
    }
  });

  const childMarkup = `<!doctype html><script>
    addEventListener("message", event => {
      const root = event.data && event.data.root;
      parent.postMessage({
        kind: "result",
        secure: isSecureContext,
        interfaceExposed: "FileSystemHandle" in globalThis,
        directoryInterfaceExposed: "FileSystemDirectoryHandle" in globalThis,
        directoryBrand: root instanceof FileSystemDirectoryHandle,
        rootName: root.name
      }, "*");
    });
    addEventListener("messageerror", () => {
      parent.postMessage({ kind: "messageerror", secure: isSecureContext }, "*");
    });
    parent.postMessage({ kind: "ready" }, "*");
  <\/script>`;
  frame = document.createElement("iframe");
  frame.src = URL.createObjectURL(new Blob([childMarkup], { type: "text/html" }));
  (document.body || document.documentElement || document).appendChild(frame);
  return "queued";
})()
"#,
        )
        .expect("blob iframe OPFS clone setup should evaluate");
    assert_eq!(setup, "queued");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__opfsBlobFrameCloneProbe)",
        r#"{"kind":"result","secure":true,"interfaceExposed":true,"directoryInterfaceExposed":true,"directoryBrand":true,"rootName":""}"#,
        "blob iframe OPFS clone",
    )
    .await;
}

#[tokio::test]
async fn sandboxed_blob_iframe_keeps_opaque_storage_context_for_opfs_messages() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://sandboxed-blob-frame.test/page.html",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__sandboxedBlobFrameProbe = "pending";
  let frame;
  let sourceFile;
  let sourceDirectory;
  let rejectionCount = 0;
  addEventListener("message", async event => {
    if (!event.data || event.source !== frame.contentWindow) {
      return;
    }
    if (event.data.kind === "ready") {
      const root = await navigator.storage.getDirectory();
      sourceFile = await root.getFileHandle("sequential-file", { create: true });
      sourceDirectory =
        await root.getDirectoryHandle("sequential-directory", { create: true });
      frame.contentWindow.postMessage({ kind: "handle", handle: sourceFile }, "*");
      return;
    }
    if (event.data.kind === "message") {
      globalThis.__sandboxedBlobFrameProbe = "unexpected-message";
      return;
    }
    if (event.data.kind === "messageerror") {
      rejectionCount += 1;
      if (rejectionCount === 1) {
        frame.contentWindow.postMessage(
          { kind: "handle", handle: sourceDirectory },
          "*"
        );
        return;
      }
      globalThis.__sandboxedBlobFrameProbe = JSON.stringify({
        rejectionCount,
        origin: event.data.origin,
        secure: event.data.secure,
        interfaceExposed: event.data.interfaceExposed,
        storageExposed: event.data.storageExposed,
        getDirectory: event.data.getDirectory,
        sourceStillUsable:
          sourceFile.name === "sequential-file" &&
          sourceDirectory.name === "sequential-directory"
      });
    }
  });
  const childMarkup = `<!doctype html><script>
    addEventListener("message", () => {
      parent.postMessage({ kind: "message" }, "*");
    });
    addEventListener("messageerror", async () => {
      let getDirectory;
      try {
        await navigator.storage.getDirectory();
        getDirectory = "resolved";
      } catch (error) {
        getDirectory = error && error.name;
      }
      parent.postMessage({
        kind: "messageerror",
        origin: location.origin,
        secure: isSecureContext,
        interfaceExposed: "FileSystemHandle" in globalThis,
        storageExposed: "storage" in navigator,
        getDirectory
      }, "*");
    });
    parent.postMessage({ kind: "ready" }, "*");
  <\/script>`;
  frame = document.createElement("iframe");
  frame.setAttribute("sandbox", "allow-scripts");
  frame.src = URL.createObjectURL(new Blob([childMarkup], { type: "text/html" }));
  (document.body || document.documentElement || document).appendChild(frame);
  return "queued";
})()
"#,
        )
        .expect("sandboxed blob iframe setup should evaluate");
    assert_eq!(setup, "queued");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__sandboxedBlobFrameProbe)",
        r#"{"rejectionCount":2,"origin":"null","secure":true,"interfaceExposed":true,"storageExposed":true,"getDirectory":"SecurityError","sourceStillUsable":true}"#,
        "sandboxed blob iframe OPFS rejection",
    )
    .await;
}

#[test]
fn structured_clone_preserves_quota_exceeded_error_fields_and_brand() {
    let mut vm = new_storage_test_vm("https://quota-exceeded-error-clone.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const source = new QuotaExceededError("full", {
                quota: 0.1,
                requested: 1
              });
              source.expando = "not serialized";
              const clone = structuredClone(source);
              const empty = structuredClone(new QuotaExceededError("empty"));
              return [
                clone instanceof QuotaExceededError,
                clone instanceof DOMException,
                Object.getPrototypeOf(clone) === QuotaExceededError.prototype,
                clone.name,
                clone.message,
                clone.code,
                clone.quota,
                clone.requested,
                clone.expando === undefined,
                empty.quota === null,
                empty.requested === null
              ].join("|");
            })()
            "#,
        )
        .expect("QuotaExceededError structured clone should evaluate");

    assert_eq!(
        result,
        "true|true|true|QuotaExceededError|full|22|0.1|1|true|true|true"
    );
}

#[test]
fn structured_clone_rejects_platform_objects_in_nested_graphs_without_running_their_getters() {
    let mut vm = new_storage_test_vm("https://platform-identity-clone.test/");
    let result = vm.eval(r#"
      (() => {
        const objects = [
          new URL('https://example.test/'), new URLSearchParams('a=b'), new Headers(),
          new Request('https://example.test/'), new Response('body'), new Response().headers, new FormData(),
          new Event('event'), new EventTarget(), new AbortController(), new AbortController().signal,
          navigator, performance, history, localStorage, new Highlight(), CSS.highlights, customElements, new CustomElementRegistry(),
          document, document.implementation, document.childNodes, document.children, document.querySelectorAll('*'), document.createElement('div').attributes, new XMLHttpRequest().upload, document.createElement('select'), document.createTextNode('text'),
          document.implementation.createHTMLDocument('detached').body,
          new TextEncoder(), new TextDecoder(), new TextEncoderStream(), new TextDecoderStream(),
          new CompressionStream('gzip'), new DecompressionStream('gzip'),
          new URLSearchParams().entries(), new Headers().entries(), new Highlight().values(),
        ];
        const failures = [];
        for (const [index, object] of objects.entries()) {
          let getterCalls = 0;
          Object.defineProperty(object, 'cloneProbe', {enumerable: true, get() { getterCalls++; return 1; }});
          try { structuredClone({nested: new Map([['entry', object]])}); failures.push(`${index}:accepted`); }
          catch (error) { if (error.name !== 'DataCloneError') failures.push(`${index}:${error.name}`); }
          if (getterCalls) failures.push(`${index}:getter`);
        }
        const buffer = new ArrayBuffer(4);
        try { structuredClone({buffer, rejected: objects[0]}, {transfer: [buffer]}); }
        catch (error) { if (error.name !== 'DataCloneError') failures.push('transfer-error'); }
        if (buffer.byteLength !== 4) failures.push('premature-detach');
        return failures.join('|');
      })()
    "#).expect("platform identity clone matrix should evaluate");
    assert_eq!(result, "");
}

#[test]
fn structured_clone_does_not_read_fake_private_slots_or_inherit_native_identity() {
    let mut vm = new_storage_test_vm("https://platform-identity-forgery.test/");
    let result = vm.eval(r#"
      (() => {
        let reads = 0;
        const prototype = Object.create(new URL('https://example.test/'));
        for (const key of ['__moliWebApiType', '__lmUrlHref', '__lmUrlSearchParamsPairs', '__lmFormDataEntries']) {
          Object.defineProperty(prototype, key, {get() { reads++; throw Error('must not run'); }});
        }
        const object = Object.create(prototype);
        object.value = 7;
        const clone = structuredClone(object);
        const forged = structuredClone({__moliWebApiType: 'URL', __lmUrlHref: 'public'});
        return [reads, clone.value, Object.getPrototypeOf(clone) === Object.prototype,
          forged.__moliWebApiType, forged.__lmUrlHref].join('|');
      })()
    "#).expect("ordinary records must not trigger native identity checks through author properties");
    assert_eq!(result, "0|7|true|URL|public");
}

#[test]
fn encoding_stream_wrappers_do_not_inherit_transform_transferability() {
    let mut vm = new_storage_test_vm("https://platform-transfer-types.test/");
    let result = vm.eval(r#"
      [new TextEncoderStream(), new TextDecoderStream(), new CompressionStream('gzip'), new DecompressionStream('gzip')]
        .map(stream => { try { structuredClone(stream, {transfer: [stream]}); return 'accepted'; }
          catch (error) { return error.name; } }).join('|')
    "#).expect("only actual transferable stream interfaces should be accepted");
    assert_eq!(
        result,
        "DataCloneError|DataCloneError|DataCloneError|DataCloneError"
    );
}

#[test]
fn shared_array_buffers_fail_consistently_at_root_and_inside_message_graphs() {
    let mut vm = new_storage_test_vm("https://shared-buffer-clone.test/");
    let result = vm.eval(r#"
      (() => {
        const shared = new SharedArrayBuffer(8);
        const channel = new MessageChannel();
        const probe = fn => { try { fn(); return 'accepted'; } catch (error) { return error.name; } };
        return [
          probe(() => structuredClone(shared)),
          probe(() => structuredClone({nested: new Map([['shared', shared]])})),
          probe(() => channel.port1.postMessage({shared})),
          probe(() => window.postMessage({shared}, '*')),
          shared.byteLength
        ].join('|');
      })()
    "#).expect("unsupported shared buffers must report DataCloneError at every depth");
    assert_eq!(
        result,
        "DataCloneError|DataCloneError|DataCloneError|DataCloneError|8"
    );
}

#[test]
fn supported_platform_codecs_preserve_payloads_after_prototype_changes() {
    let mut vm = new_storage_test_vm("https://supported-platform-codecs.test/");
    let result = vm.eval(r#"
      (() => {
        const blob = new (class extends Blob {})(['abc'], {type: 'text/plain'});
        const file = new File(['def'], 'note.txt', {type: 'text/plain', lastModified: 17});
        const pixels = new ImageData(new Uint8ClampedArray([1, 2, 3, 4]), 1, 1);
        const exception = new DOMException('message', 'AbortError');
        let getterCalls = 0;
        for (const object of [blob, file, pixels, exception]) {
          Object.setPrototypeOf(object, null);
          Object.defineProperty(object, 'expando', {enumerable: true, get() { getterCalls++; return 1; }});
        }
        const clone = structuredClone({blob, file, pixels, exception, again: blob});
        return [clone.blob instanceof Blob, clone.blob.size, clone.blob.type, clone.blob === clone.again,
          clone.file instanceof File, clone.file.name, clone.file.lastModified,
          clone.pixels instanceof ImageData, Array.from(clone.pixels.data).join(','),
          clone.exception instanceof DOMException, clone.exception.name, clone.exception.message,
          getterCalls, clone.blob.expando === undefined].join('|');
      })()
    "#).expect("native codecs should ignore mutable prototypes and author expandos");
    assert_eq!(
        result,
        "true|3|text/plain|true|true|note.txt|17|true|1,2,3,4|true|AbortError|message|0|true"
    );
}

#[test]
fn observer_and_webgl_factory_results_have_native_identity() {
    let mut vm = new_storage_test_vm("https://factory-result-identity.test/");
    let result = vm
        .eval(
            r#"
      (() => {
        const target = document.createElement('div');
        target.style.cssText = 'width: 41px; height: 23px';
        const html = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || html.appendChild(document.createElement('body'));
        body.appendChild(target);
        const observer = new ResizeObserver(() => {});
        observer.observe(target);
        const entry = observer.takeRecords()[0];
        const gl = document.createElement('canvas').getContext('webgl');
        const precision = gl.getShaderPrecisionFormat(gl.VERTEX_SHADER, gl.HIGH_FLOAT);
        return [entry, entry.contentBoxSize[0], entry.borderBoxSize[0], precision].map(value => {
          try { structuredClone({value}); return 'accepted'; }
          catch (error) { return error.name; }
        }).join('|');
      })()
    "#,
        )
        .expect("factory results must be identified before structured clone");
    assert_eq!(
        result,
        "DataCloneError|DataCloneError|DataCloneError|DataCloneError"
    );
}
