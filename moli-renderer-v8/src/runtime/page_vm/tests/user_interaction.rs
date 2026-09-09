use super::*;

use crate::page_task_queue::{
    PageUserInteractionTargetEffect, RendererOwnerWakeSource, RendererPageReadyDescriptor,
    RendererPageSchedulerTask, RendererPageUserInteractionTask,
};

fn take_next_user_interaction_task_for_authorization_test(
    page_vm: &mut PageVm,
) -> Option<RendererPageUserInteractionTask> {
    let sources = page_vm.page_task_executor_sources_for_test();
    let task = sources.take_scheduler_task_for_executor_test(|descriptor| {
        let RendererPageReadyDescriptor::UserInteraction { .. } = descriptor else {
            return false;
        };
        true
    })?;
    let RendererPageSchedulerTask::UserInteraction(task) = task else {
        unreachable!("user-interaction descriptor must dequeue its own task")
    };
    Some(task)
}

#[tokio::test(flavor = "current_thread")]
async fn user_interaction_body_cleans_up_callbacks_before_selected_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/user-interaction-body-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__userInteractionBodyBoundary = [];
const dialog = document.createElement("dialog");
dialog.addEventListener("close", () => {
  __userInteractionBodyBoundary.push("callback");
  Promise.resolve().then(() => __userInteractionBodyBoundary.push("microtask"));
});
document.body.append(dialog);
dialog.show();
dialog.close();
"queued"
"#,
        )?;

        let task = take_next_user_interaction_task_for_authorization_test(&mut page_vm)
            .expect("one exact user-interaction task should be ready");
        let body = page_vm.apply_selected_page_user_interaction_turn(task)?;
        assert_eq!(
            body.action.target_effect,
            PageUserInteractionTargetEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__userInteractionBodyBoundary.join('|')")?,
            "callback|microtask",
            "listener cleanup must drain reactions before the selected task completes"
        );

        page_vm.finish_selected_page_callback_task(&loader).await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__userInteractionBodyBoundary.join('|')")?,
            "callback|microtask",
            "selected task completion must not repeat callback reactions or their inline scripts"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("user-interaction body/completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn user_interaction_completion_syncs_a_microtask_created_child_after_the_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/user-interaction-microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__userInteractionChildOrder = [];
const dialog = document.createElement("dialog");
dialog.addEventListener("close", () => {
  __userInteractionChildOrder.push("callback");
  Promise.resolve().then(() => {
    __userInteractionChildOrder.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "user-interaction-microtask-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
  });
});
document.body.append(dialog);
dialog.show();
dialog.close();
"queued"
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::UserInteraction, &loader)
                .await?,
            "the exact user-interaction task should run through the selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__userInteractionChildOrder.join('|')")?,
            "callback|microtask",
            "the agent checkpoint must precede callback child-record synchronization"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "a reaction-created srcdoc frame must publish a typed navigation commit during callback completion"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit),
            "the microtask-created frame must remain a concrete later Page task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("user-interaction post-checkpoint child synchronization test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn data_transfer_get_as_string_uses_one_typed_user_interaction_task_per_callback() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/data-transfer-string-callback").unwrap();
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        while owner_wake_rx.try_recv().is_ok() {}

        let initial = page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__dataTransferStringEvents = [];
  globalThis.__dataTransferStringErrors = [];
  const conversion = [];
  const transfer = new DataTransfer();
  const stringItem = transfer.items.add("alpha", "text/plain");
  const fileItem = transfer.items.add(new File(["file"], "fixture.txt"));
  const disabledTransfer = new DataTransfer();
  const disabledItem = disabledTransfer.items.add("disabled", "text/plain");
  disabledTransfer.items.clear();

  for (const [label, invoke] of [
    ["missing", () => stringItem.getAsString()],
    ["undefined", () => stringItem.getAsString(undefined)],
    ["null", () => stringItem.getAsString(null)],
    ["object", () => stringItem.getAsString({})]
  ]) {
    try {
      invoke();
      conversion.push(`${label}:ok`);
    } catch (error) {
      conversion.push(`${label}:${error.name}`);
    }
  }

  let fileCallbackCalled = false;
  let disabledCallbackCalled = false;
  fileItem.getAsString(() => { fileCallbackCalled = true; });
  disabledItem.getAsString(() => { disabledCallbackCalled = true; });

  onerror = (_message, _source, _line, _column, error) => {
    __dataTransferStringErrors.push(error && error.name);
    return true;
  };
  const callback = new Proxy(
    function(value) {
      "use strict";
      __dataTransferStringEvents.push(
        `callback:${this === undefined}:${arguments.length}:${value}`
      );
      Promise.resolve().then(() => {
        __dataTransferStringEvents.push(`microtask:${value}`);
      });
    },
    {
      apply(target, receiver, args) {
        __dataTransferStringEvents.push(
          `apply:${receiver === undefined}:${args.length}`
        );
        return Reflect.apply(target, receiver, args);
      }
    }
  );
  stringItem.getAsString(callback);

  const revokedItem = transfer.items.add("beta", "text/html");
  const revoked = Proxy.revocable(function() {}, {});
  revoked.revoke();
  let revokedAccepted = true;
  try {
    revokedItem.getAsString(revoked.proxy);
  } catch {
    revokedAccepted = false;
  }

  transfer.setData("text/plain", "mutated-after-admission");
  transfer.clearData("text/html");
  __dataTransferStringEvents.push("sync");
  return JSON.stringify({
    conversion,
    fileCallbackCalled,
    disabledCallbackCalled,
    revokedAccepted,
    events: [...__dataTransferStringEvents]
  });
})()
"#,
        )?;
        assert_eq!(
            initial,
            r#"{"conversion":["missing:TypeError","undefined:ok","null:ok","object:TypeError"],"fileCallbackCalled":false,"disabledCallbackCalled":false,"revokedAccepted":true,"events":["sync"]}"#
        );

        let wakes = std::iter::from_fn(|| owner_wake_rx.try_recv().ok())
            .map(|wake| wake.source_for_test())
            .collect::<Vec<_>>();
        assert_eq!(
            wakes,
            vec![RendererOwnerWakeSource::UserInteractionTask],
            "the empty-to-nonempty transition must publish one admission-only wake"
        );
        assert!(
            !page_vm.vm().has_ready_timeout(),
            "getAsString must not acquire a PageTimer descriptor"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::UserInteraction,
                    &loader,
                )
                .await?,
            "the first callback must run through the production selected-task dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__dataTransferStringEvents)")?,
            r#"["sync","apply:true:1","callback:true:1:alpha","microtask:alpha"]"#,
            "admission must freeze one string and selected completion must own its checkpoint"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::UserInteraction,
                    &loader,
                )
                .await?,
            "the revoked callable Proxy must remain an admitted callback task"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__dataTransferStringErrors)")?,
            r#"["TypeError"]"#,
            "invocation failure must be reported through the callback Window"
        );
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::UserInteraction,
                    &loader,
                )
                .await?,
            "null, file, and disabled items must not manufacture callback tasks"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("typed DataTransfer string callback test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn data_transfer_get_as_string_separates_calling_target_and_callback_realms() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/data-transfer-string-realms").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__dataTransferRealmEvents = [];
globalThis.__dataTransferWrongParentErrors = 0;
onerror = () => {
  __dataTransferWrongParentErrors += 1;
  return true;
};
const frame = document.createElement("iframe");
frame.id = "data-transfer-string-realm";
document.body.appendChild(frame);
"created"
"#,
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("data-transfer-string-realm")
            .expect("callback Realm fixture should expose its child handle");
        let child_context =
            materialize_only_child_realm_execution_context_through_page_turn_for_test(
                &mut page_vm,
                "data-transfer-string-realm",
            )?;
        page_vm.vm_mut().eval_in_child_default_context(
            child_context,
            r#"
onerror = (_message, _source, _line, _column, error) => {
  parent.__dataTransferRealmEvents.push(`child-error:${error && error.message}`);
  return true;
};
globalThis.__dataTransferChildCallback = function(value) {
  "use strict";
  parent.__dataTransferRealmEvents.push(
    `child-callback:${globalThis === parent.document.getElementById("data-transfer-string-realm").contentWindow}:${this === undefined}:${arguments.length}:${value}`
  );
  throw new Error("child-data-transfer");
};
globalThis.__dataTransferRetiredCallback = function() {
  parent.__dataTransferRealmEvents.push("retired-callback-ran");
};
"installed"
"#,
        )?;

        page_vm.vm_mut().eval(
            r#"
const crossRealmTransfer = new DataTransfer();
crossRealmTransfer.items
  .add("cross-realm", "text/plain")
  .getAsString(
    document.getElementById("data-transfer-string-realm")
      .contentWindow.__dataTransferChildCallback
  );
"queued"
"#,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::UserInteraction,
                    &loader,
                )
                .await?,
            "the cross-Realm callback task should run"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__dataTransferRealmEvents)")?,
            r#"["child-callback:true:true:1:cross-realm","child-error:child-data-transfer"]"#,
            "callback invocation and exception reporting must use the callback Realm"
        );
        assert_eq!(
            page_vm.vm_mut().eval("String(__dataTransferWrongParentErrors)")?,
            "0"
        );

        page_vm.vm_mut().eval(
            r#"
const retiredCallbackTransfer = new DataTransfer();
retiredCallbackTransfer.items
  .add("retired", "text/plain")
  .getAsString(
    document.getElementById("data-transfer-string-realm")
      .contentWindow.__dataTransferRetiredCallback
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
globalThis.__dataTransferRetiredCheckpoint = 0;
Promise.resolve().then(() => { __dataTransferRetiredCheckpoint += 1; });
"queued"
"#,
            )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::UserInteraction,
                    &loader,
                )
                .await?,
            "the current calling target must settle a retired callback task"
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(__dataTransferRetiredCheckpoint)",
            )?,
            "1",
            "a current target with no runnable callback still owns one task checkpoint"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__dataTransferRealmEvents)")?,
            r#"["child-callback:true:true:1:cross-realm","child-error:child-data-transfer"]"#,
            "retiring the callback Realm must suppress the callback"
        );

        let replacement_context =
            materialize_only_child_realm_execution_context_through_page_turn_for_test(
                &mut page_vm,
                "data-transfer-string-realm",
            )?;
        page_vm.vm_mut().eval(
            r#"
globalThis.__dataTransferTargetEvents = [];
globalThis.__dataTransferParentCallback = value => {
  __dataTransferTargetEvents.push(value);
};
"installed"
"#,
        )?;
        page_vm.vm_mut().eval_in_child_default_context(
            replacement_context,
            r#"
const targetTransfer = new DataTransfer();
targetTransfer.items
  .add("retired-target", "text/plain")
  .getAsString(parent.__dataTransferParentCallback);
"queued"
"#,
        )?;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::UserInteraction)
            .expect("the exact child-target callback task should be claimable");
        page_vm.vm_mut().eval(
            r#"
document.getElementById("data-transfer-string-realm").remove();
"removed"
"#,
        )?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__dataTransferStaleTargetCheckpoint = 0;
Promise.resolve().then(() => { __dataTransferStaleTargetCheckpoint += 1; });
"queued"
"#,
            )?;
        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(__dataTransferStaleTargetCheckpoint)",
            )?,
            "0",
            "a retired calling target must not checkpoint an unrelated live Realm"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "JSON.stringify(__dataTransferTargetEvents)",
                )?,
            "[]",
            "a retired calling target must discard its exact callback task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("DataTransfer calling-target/callback-Realm test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn user_interaction_family_preserves_cross_api_fifo_and_one_checkpoint_per_turn() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/user-interaction-fifo").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__userInteractionLog = [];
const input = document.createElement("input");
input.value = "abcd";
for (const type of ["select", "selectionchange"]) {
  input.addEventListener(type, () => {
    __userInteractionLog.push(`input:${type}`);
    Promise.resolve().then(() => __userInteractionLog.push(`microtask:${type}`));
  });
}
document.body.append(input);
input.setSelectionRange(0, 2);

const transfer = new DataTransfer();
transfer.items.add("alpha", "text/plain").getAsString(value => {
  __userInteractionLog.push(`data-transfer:${value}`);
  Promise.resolve().then(() => __userInteractionLog.push("microtask:data-transfer"));
});

const dialog = document.createElement("dialog");
dialog.addEventListener("close", () => {
  __userInteractionLog.push("dialog:close");
  Promise.resolve().then(() => __userInteractionLog.push("microtask:close"));
});
document.body.append(dialog);
dialog.show();
dialog.close();

const text = document.createTextNode("selection");
document.body.append(text);
document.addEventListener("selectionchange", event => {
  if (event.target === document) {
    __userInteractionLog.push("document:selectionchange");
    Promise.resolve().then(() => __userInteractionLog.push("microtask:document"));
  }
});
getSelection().collapse(text, 1);
"queued"
"#,
        )?;

        assert!(
            !page_vm.vm().has_ready_timeout(),
            "user-interaction tasks must not acquire PageTimer descriptors"
        );
        let expected_after_each_turn = [
            "input:select|microtask:select",
            "input:select|microtask:select|input:selectionchange|microtask:selectionchange",
            "input:select|microtask:select|input:selectionchange|microtask:selectionchange|data-transfer:alpha|microtask:data-transfer",
            "input:select|microtask:select|input:selectionchange|microtask:selectionchange|data-transfer:alpha|microtask:data-transfer|dialog:close|microtask:close",
            "input:select|microtask:select|input:selectionchange|microtask:selectionchange|data-transfer:alpha|microtask:data-transfer|dialog:close|microtask:close|document:selectionchange|microtask:document",
        ];
        for expected in expected_after_each_turn {
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::UserInteraction, &loader)
                    .await?,
                "one family task should remain queued"
            );
            assert_eq!(
                page_vm.vm_mut().eval("__userInteractionLog.join('|')")?,
                expected
            );
        }
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::UserInteraction, &loader)
                .await?,
            "five API callbacks/events must consume exactly five browser task turns"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("user-interaction FIFO test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn user_interaction_task_is_document_exact_across_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/user-interaction-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let before_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");

        page_vm.vm_mut().eval(
            r#"
const text = document.createTextNode("abcd");
document.body.append(text);
document.addEventListener("selectionchange", () => {});
getSelection().collapse(text, 1);
document.open();
document.write("<!doctype html><body>replacement</body>");
document.close();
"replaced"
"#,
        )?;
        let after_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner should exist");
        assert_ne!(before_document, after_document);
        assert_eq!(
            before_document.local_window_id,
            after_document.local_window_id
        );

        let task = take_next_user_interaction_task_for_authorization_test(&mut page_vm)
            .expect("retired Document user-interaction task should settle");
        let stale = page_vm.apply_selected_page_user_interaction_turn(task)?;
        let PageUserInteractionTargetEffect::DiscardedStaleOwner {
            current_owner: Some(current_owner),
        } = stale.action.target_effect
        else {
            panic!("stale task should report the replacement Document owner")
        };
        assert_ne!(stale.action.owner, current_owner);
        assert!(
            take_next_user_interaction_task_for_authorization_test(&mut page_vm).is_none(),
            "stale settlement must retire the Host-local pending slot"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Document-exact user-interaction test should run");
}
