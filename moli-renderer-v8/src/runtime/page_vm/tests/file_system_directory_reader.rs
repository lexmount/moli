use super::*;
use crate::page_task_queue::RendererOwnerWakeSource;
use crate::{RendererDragData, RendererDraggedDirectory, RendererDraggedFile};

fn directory_drag_data(file_count: usize) -> RendererDragData {
    RendererDragData {
        items: Vec::new(),
        files: Vec::new(),
        directories: vec![RendererDraggedDirectory {
            name: "fixture".to_owned(),
            files: (0..file_count)
                .map(|index| RendererDraggedFile {
                    bytes: format!("file-{index}").into_bytes(),
                    mime_type: "text/plain".to_owned(),
                    name: format!("entry-{index:03}.txt"),
                    last_modified: index as f64,
                })
                .collect(),
            directories: Vec::new(),
        }],
        drag_operations_mask: 1,
    }
}

fn install_directory_reader(page_vm: &mut PageVm, file_count: usize) -> anyhow::Result<()> {
    page_vm.vm_mut().eval(
        r#"
const dropTarget = document.createElement("div");
dropTarget.id = "directory-reader-drop-target";
dropTarget.style.width = "100px";
dropTarget.style.height = "100px";
dropTarget.addEventListener("drop", event => {
  globalThis.__directoryEntry =
    event.dataTransfer.items[0].webkitGetAsEntry();
  globalThis.__directoryReader = __directoryEntry.createReader();
});
document.body.appendChild(dropTarget);
"installed"
"#,
    )?;
    let outcome = page_vm.dispatch_drag_event_at_point(
        10.0,
        10.0,
        "drop",
        directory_drag_data(file_count),
        0,
    )?;
    assert!(outcome.handled, "the directory drop fixture must dispatch");
    assert_eq!(
        page_vm.vm_mut().eval(
            "__directoryEntry.isDirectory && \
             __directoryReader instanceof FileSystemDirectoryReader",
        )?,
        "true"
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn directory_reader_uses_file_reading_batches_and_overlap_semantics() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/directory-reader-file-reading").unwrap();
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_directory_reader(&mut page_vm, 102)?;
        while owner_wake_rx.try_recv().is_ok() {}

        let initial = page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__directoryReaderEvents = [];
  globalThis.__directoryReaderErrors = [];
  Object.assign(__directoryReader, {
    __lmFileSystemDirectoryReaderEntries: [],
    __lmFileSystemDirectoryReaderOffset: 102,
    __lmFileSystemDirectoryReaderActiveRequest: 99n,
    __lmFileSystemDirectoryReaderDone: true,
    __lmFileSystemDirectoryReaderError:
      new DOMException("spoofed", "NotReadableError")
  });
  const conversions = [];
  for (const [label, invoke] of [
    ["missing", () => __directoryReader.readEntries()],
    ["undefined", () => __directoryReader.readEntries(undefined)],
    ["null", () => __directoryReader.readEntries(null)],
    ["object", () => __directoryReader.readEntries({})],
    ["null-error", () => __directoryReader.readEntries(() => {}, null)],
    ["object-error", () => __directoryReader.readEntries(() => {}, {})]
  ]) {
    try {
      invoke();
      conversions.push(`${label}:ok`);
    } catch (error) {
      conversions.push(`${label}:${error.name}`);
    }
  }

  globalThis.__directorySuccess = new Proxy(
    function(entries) {
      "use strict";
      const first = entries.length ? entries[0].name : "-";
      const last = entries.length ? entries[entries.length - 1].name : "-";
      __directoryReaderEvents.push(
        `batch:${this === undefined}:${arguments.length}:` +
        `${entries.length}:${first}:${last}`
      );
      Promise.resolve().then(() => {
        __directoryReaderEvents.push(`microtask:${entries.length}`);
      });
    },
    {
      apply(target, receiver, argumentsList) {
        __directoryReaderEvents.push(
          `apply:${receiver === undefined}:${argumentsList.length}`
        );
        return Reflect.apply(target, receiver, argumentsList);
      }
    }
  );
  globalThis.__directoryError = new Proxy(
    function(error) {
      "use strict";
      __directoryReaderErrors.push(
        `error:${this === undefined}:${arguments.length}:` +
        `${error.name}:${error instanceof DOMException}`
      );
      Promise.resolve().then(() => {
        __directoryReaderErrors.push("microtask:error");
      });
    },
    {
      apply(target, receiver, argumentsList) {
        __directoryReaderErrors.push(
          `apply:${receiver === undefined}:${argumentsList.length}`
        );
        return Reflect.apply(target, receiver, argumentsList);
      }
    }
  );

  const firstReturn = __directoryReader.readEntries(__directorySuccess);
  const ignoredOverlapReturn = __directoryReader.readEntries(() => {
    __directoryReaderEvents.push("unexpected-overlap-success");
  });
  const reportedOverlapReturn =
    __directoryReader.readEntries(() => {
      __directoryReaderEvents.push("unexpected-reported-overlap-success");
    }, __directoryError);
  __directoryReaderEvents.push("sync");
  return JSON.stringify({
    conversions,
    returns: [
      firstReturn === undefined,
      ignoredOverlapReturn === undefined,
      reportedOverlapReturn === undefined
    ],
    events: __directoryReaderEvents,
    errors: __directoryReaderErrors
  });
})()
"#,
        )?;
        assert_eq!(
            initial,
            r#"{"conversions":["missing:TypeError","undefined:TypeError","null:TypeError","object:TypeError","null-error:TypeError","object-error:TypeError"],"returns":[true,true,true],"events":["sync"],"errors":[]}"#,
            "callback conversion must finish before reader state inspection or task admission"
        );

        let wakes = std::iter::from_fn(|| owner_wake_rx.try_recv().ok())
            .map(|wake| wake.source_for_test())
            .collect::<Vec<_>>();
        assert_eq!(
            wakes,
            vec![RendererOwnerWakeSource::FileReadingTask],
            "one nonempty FileReading epoch must publish exactly one owner wake"
        );
        assert!(
            !page_vm.vm().has_ready_timeout(),
            "readEntries must not acquire a PageTimer descriptor"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::FileReading)
                .await?,
            "the first batch must run through the production selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__directoryReaderEvents.join('|')")?,
            "sync|apply:true:1|batch:true:1:100:entry-000.txt:entry-099.txt|microtask:100"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::FileReading)
                .await?,
            "the admitted overlap error must preserve FileReading FIFO"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__directoryReaderErrors.join('|')")?,
            "apply:true:1|error:true:1:InvalidStateError:true|microtask:error",
            "overlap must report asynchronously without disturbing the first request"
        );
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::FileReading)
                .await?,
            "missing optional error callback and conversion failures must publish no tasks"
        );

        for expected in [
            "batch:true:1:2:entry-100.txt:entry-101.txt",
            "batch:true:1:0:-:-",
            "batch:true:1:0:-:-",
        ] {
            page_vm
                .vm_mut()
                .eval("__directoryReader.readEntries(__directorySuccess); 'queued'")?;
            assert_eq!(
                owner_wake_rx
                    .try_recv()
                    .expect("an empty FileReading source must publish a new readiness edge")
                    .source_for_test(),
                RendererOwnerWakeSource::FileReadingTask
            );
            assert!(
                owner_wake_rx.try_recv().is_err(),
                "one request must not duplicate its readiness edge"
            );
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::FileReading)
                    .await?
            );
            let tail = page_vm.vm_mut().eval(
                "__directoryReaderEvents.slice(-3, -1).join('|')",
            )?;
            assert_eq!(
                tail,
                format!("apply:true:1|{expected}"),
                "the reader must deliver 100/2/empty batches and remain asynchronously empty after done"
            );
        }

        assert_eq!(
            page_vm.vm_mut().eval(
                "__directoryReaderEvents.filter(value => \
                 value.startsWith('batch:')).map(value => \
                 value.split(':')[3]).join(',')",
            )?,
            "100,2,0,0",
            "done becomes observable only through the first selected empty batch"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("typed directory-reader FileReading test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn directory_reader_separates_calling_target_and_callback_realms() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/directory-reader-realms").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_directory_reader(&mut page_vm, 3)?;

        page_vm.vm_mut().eval(
            r#"
globalThis.__directoryRealmEvents = [];
globalThis.__directoryParentErrors = 0;
onerror = () => {
  __directoryParentErrors += 1;
  return true;
};
const frame = document.createElement("iframe");
frame.id = "directory-reader-callback-realm";
document.body.appendChild(frame);
"created"
"#,
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("directory-reader-callback-realm")
            .expect("callback Realm fixture should expose its child handle");
        let child_context =
            materialize_only_child_realm_execution_context_through_page_turn_for_test(
                &mut page_vm,
                "directory-reader-callback-realm",
            )?;
        page_vm.vm_mut().eval_in_child_default_context(
            child_context,
            r#"
onerror = (_message, _source, _line, _column, error) => {
  parent.__directoryRealmEvents.push(`child-error:${error && error.message}`);
  return true;
};
globalThis.__directoryChildCallback = function(entries) {
  "use strict";
  parent.__directoryRealmEvents.push(
    `child:${globalThis === parent.document.getElementById("directory-reader-callback-realm").contentWindow}:` +
    `${this === undefined}:${arguments.length}:` +
    `${Object.getPrototypeOf(entries) === Array.prototype}:${entries.length}`
  );
  throw new Error("child-directory-reader");
};
globalThis.__directoryRetiredCallback = function() {
  parent.__directoryRealmEvents.push("retired-callback-ran");
};
"installed"
"#,
        )?;

        page_vm.vm_mut().eval(
            r#"
__directoryReader.readEntries(
  document.getElementById("directory-reader-callback-realm")
    .contentWindow.__directoryChildCallback
);
"queued"
"#,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::FileReading)
                .await?
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__directoryRealmEvents)")?,
            r#"["child:true:true:1:true:3","child-error:child-directory-reader"]"#,
            "argument creation, invocation, and exception reporting must use the callback Realm"
        );
        assert_eq!(
            page_vm.vm_mut().eval("String(__directoryParentErrors)")?,
            "0"
        );

        page_vm.vm_mut().eval(
            r#"
__directoryReader.readEntries(
  document.getElementById("directory-reader-callback-realm")
    .contentWindow.__directoryRetiredCallback
);
"queued"
"#,
        )?;
        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(child_handle);
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__directoryRetiredCheckpoint = 0;
Promise.resolve().then(() => { __directoryRetiredCheckpoint += 1; });
"queued"
"#,
            )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::FileReading)
                .await?
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(__directoryRetiredCheckpoint)",
            )?,
            "1",
            "a current target with a retired callback Realm still owns one checkpoint"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__directoryRealmEvents)")?,
            r#"["child:true:true:1:true:3","child-error:child-directory-reader"]"#,
            "retiring the callback Realm must suppress invocation"
        );

        page_vm.vm_mut().eval(
            r#"
globalThis.__directoryStaleEvents = [];
globalThis.__directoryStaleReader = __directoryEntry.createReader();
__directoryStaleReader.readEntries(entries => {
  __directoryStaleEvents.push(entries.length);
});
"queued"
"#,
        )?;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::FileReading,
            )
            .expect("the exact directory-reader task should be claimable");
        page_vm.vm_mut().eval(
            r#"
document.open();
document.write("<!doctype html><body>replacement</body>");
document.close();
"replaced"
"#,
        )?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__directoryStaleCheckpoint = 0;
Promise.resolve().then(() => { __directoryStaleCheckpoint += 1; });
"queued"
"#,
            )?;
        page_vm
            .run_claimed_selected_page_task_for_test(claimed)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(__directoryStaleCheckpoint)",
            )?,
            "0",
            "a stale calling Document must not checkpoint the replacement"
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "JSON.stringify(__directoryStaleEvents)",
            )?,
            "[]",
            "a stale calling Document must not invoke its callback"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("directory-reader calling-target/callback-Realm test should run");
}
