use super::*;

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
