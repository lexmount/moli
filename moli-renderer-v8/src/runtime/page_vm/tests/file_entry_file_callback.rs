use super::*;
use crate::page_task_queue::RendererOwnerWakeSource;

#[tokio::test(flavor = "current_thread")]
async fn file_entry_file_uses_webidl_conversion_and_the_shared_dom_task_source() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/file-entry-file-callback").unwrap();
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        while owner_wake_rx.try_recv().is_ok() {}

        let initial = page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__fileEntryFileEvents = [];
  const originalFile = new File(["body"], "fixture.txt", {
    type: "text/plain"
  });
  globalThis.__fileEntryOriginalFile = originalFile;
  const transfer = new DataTransfer();
  const entry = transfer.items.add(originalFile).webkitGetAsEntry();
  globalThis.__fileEntryFixture = entry;

  const conversions = [];
  for (const [label, invoke] of [
    ["missing", () => entry.file()],
    ["undefined-success", () => entry.file(undefined)],
    ["null-success", () => entry.file(null)],
    ["object-success", () => entry.file({})],
    ["null-error", () => entry.file(() => {}, null)],
    ["object-error", () => entry.file(() => {}, {})]
  ]) {
    try {
      invoke();
      conversions.push(`${label}:ok`);
    } catch (error) {
      conversions.push(`${label}:${error.name}`);
    }
  }

  globalThis.__fileEntryBroadcastReceiver =
    new BroadcastChannel("file-entry-file-callback");
  __fileEntryBroadcastReceiver.onmessage = () => {
    __fileEntryFileEvents.push("broadcast");
    Promise.resolve().then(() => {
      __fileEntryFileEvents.push("microtask:broadcast");
    });
  };
  globalThis.__fileEntryBroadcastSender =
    new BroadcastChannel("file-entry-file-callback");
  __fileEntryBroadcastSender.postMessage("first");

  const callback = new Proxy(
    function(file) {
      "use strict";
      __fileEntryFileEvents.push(
        `file:${this === undefined}:${arguments.length}:` +
        `${file === __fileEntryOriginalFile}:${file.name}:${file.type}:${file.size}`
      );
      Promise.resolve().then(() => {
        __fileEntryFileEvents.push("microtask:file");
      });
    },
    {
      apply(target, receiver, argumentsList) {
        __fileEntryFileEvents.push(
          `apply:${receiver === undefined}:${argumentsList.length}`
        );
        return Reflect.apply(target, receiver, argumentsList);
      }
    }
  );
  const returned = entry.file(callback, undefined);
  __fileEntryFileEvents.push(`sync:${returned === undefined}`);
  return JSON.stringify({ conversions, events: __fileEntryFileEvents });
})()
"#,
        )?;
        assert_eq!(
            initial,
            r#"{"conversions":["missing:TypeError","undefined-success:TypeError","null-success:TypeError","object-success:TypeError","null-error:TypeError","object-error:TypeError"],"events":["sync:true"]}"#,
            "required success and optional non-nullable error callbacks must convert before task admission"
        );

        let wakes = std::iter::from_fn(|| owner_wake_rx.try_recv().ok())
            .map(|wake| wake.source_for_test())
            .collect::<Vec<_>>();
        assert_eq!(
            wakes,
            vec![RendererOwnerWakeSource::DomManipulationTask],
            "BroadcastChannel and file() must share one empty-to-nonempty DOM-source wake"
        );
        assert!(
            !page_vm.vm().has_ready_timeout(),
            "file() must not acquire a PageTimer descriptor"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::BroadcastChannel,
                    ))
                .await?,
            "the earlier BroadcastChannel task must remain the shared FIFO head"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__fileEntryFileEvents.join('|')")?,
            "sync:true|broadcast|microtask:broadcast"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::FileEntryFileCallback,
                    ))
                .await?,
            "file() must run through the production selected-task dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__fileEntryFileEvents.join('|')")?,
            "sync:true|broadcast|microtask:broadcast|apply:true:1|file:true:1:true:fixture.txt:text/plain:4|microtask:file",
            "the task must retain the exact File, invoke with undefined, and complete its checkpoint"
        );
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::FileEntryFileCallback,
                    ))
                .await?,
            "failed callback conversions must not manufacture hidden tasks"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("typed FileSystemFileEntry.file callback test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn file_entry_file_separates_calling_target_and_callback_realms() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/file-entry-file-realms").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__fileEntryRealmEvents = [];
globalThis.__fileEntryWrongParentErrors = 0;
onerror = () => {
  __fileEntryWrongParentErrors += 1;
  return true;
};
const frame = document.createElement("iframe");
frame.id = "file-entry-file-realm";
document.body.appendChild(frame);
"created"
"#,
        )?;
        let child_context =
            materialize_only_child_realm_execution_context_through_page_turn_for_test(
                &mut page_vm,
                "file-entry-file-realm",
            )?;
        page_vm.vm_mut().eval_in_child_default_context(
            child_context,
            r#"
onerror = (_message, _source, _line, _column, error) => {
  parent.__fileEntryRealmEvents.push(`child-error:${error && error.message}`);
  return true;
};
globalThis.__fileEntryChildCallback = function(file) {
  "use strict";
  parent.__fileEntryRealmEvents.push(
    `child:${globalThis === parent.document.getElementById("file-entry-file-realm").contentWindow}:` +
    `${this === undefined}:${arguments.length}:${file.name}`
  );
  throw new Error("child-file-entry");
};
globalThis.__fileEntryRetiredCallback = function() {
  parent.__fileEntryRealmEvents.push("retired-callback-ran");
};
"installed"
"#,
        )?;

        page_vm.vm_mut().eval(
            r#"
const crossRealmFile = new File(["cross"], "cross.txt");
const crossRealmTransfer = new DataTransfer();
crossRealmTransfer.items
  .add(crossRealmFile)
  .webkitGetAsEntry()
  .file(
    document.getElementById("file-entry-file-realm")
      .contentWindow.__fileEntryChildCallback
  );
"queued"
"#,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::FileEntryFileCallback,
                    ))
                .await?,
            "the cross-Realm callback task should run"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__fileEntryRealmEvents)")?,
            r#"["child:true:true:1:cross.txt","child-error:child-file-entry"]"#,
            "callback invocation and exception reporting must use the callback Realm"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(__fileEntryWrongParentErrors)")?,
            "0"
        );

        page_vm.vm_mut().eval(
            r#"
const retiredCallbackFile = new File(["retired"], "retired.txt");
const retiredCallbackTransfer = new DataTransfer();
retiredCallbackTransfer.items
  .add(retiredCallbackFile)
  .webkitGetAsEntry()
  .file(
    document.getElementById("file-entry-file-realm")
      .contentWindow.__fileEntryRetiredCallback
  );
"queued"
"#,
        )?;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::DomManipulation(
                    PageDomManipulationTestFamily::FileEntryFileCallback,
                ),
            )
            .expect("the current-target callback task should be claimable");
        page_vm.vm_mut().eval(
            r#"document.getElementById("file-entry-file-realm").remove(); "removed""#,
        )?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__fileEntryRetiredCheckpoint = 0;
Promise.resolve().then(() => { __fileEntryRetiredCheckpoint += 1; });
"queued"
"#,
            )?;
        page_vm
            .run_claimed_selected_page_task_for_test(claimed)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(__fileEntryRetiredCheckpoint)",
            )?,
            "1",
            "a current target with a retired callback Realm still owns one checkpoint"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__fileEntryRealmEvents)")?,
            r#"["child:true:true:1:cross.txt","child-error:child-file-entry"]"#,
            "retiring the callback Realm must suppress invocation"
        );

        page_vm.vm_mut().eval(
            r#"
const targetFrame = document.createElement("iframe");
targetFrame.id = "file-entry-file-target";
document.body.appendChild(targetFrame);
globalThis.__fileEntryTargetEvents = [];
globalThis.__fileEntryParentCallback = file => {
  __fileEntryTargetEvents.push(file.name);
};
"created"
"#,
        )?;
        let target_context =
            materialize_only_child_realm_execution_context_through_page_turn_for_test(
                &mut page_vm,
                "file-entry-file-target",
            )?;
        page_vm.vm_mut().eval_in_child_default_context(
            target_context,
            r#"
const targetFile = new File(["target"], "target.txt");
const targetTransfer = new DataTransfer();
targetTransfer.items
  .add(targetFile)
  .webkitGetAsEntry()
  .file(parent.__fileEntryParentCallback);
"queued"
"#,
        )?;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::DomManipulation(
                    PageDomManipulationTestFamily::FileEntryFileCallback,
                ),
            )
            .expect("the exact child-target callback task should be claimable");
        page_vm.vm_mut().eval(
            r#"document.getElementById("file-entry-file-target").remove(); "removed""#,
        )?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__fileEntryStaleTargetCheckpoint = 0;
Promise.resolve().then(() => { __fileEntryStaleTargetCheckpoint += 1; });
"queued"
"#,
            )?;
        page_vm
            .run_claimed_selected_page_task_for_test(claimed)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(__fileEntryStaleTargetCheckpoint)",
            )?,
            "0",
            "a retired calling target must not checkpoint an unrelated live Realm"
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "JSON.stringify(__fileEntryTargetEvents)",
            )?,
            "[]",
            "a retired calling target must discard its exact callback task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("FileSystemFileEntry calling-target/callback-Realm test should run");
}
