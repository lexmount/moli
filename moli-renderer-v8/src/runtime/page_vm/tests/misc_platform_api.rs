use super::*;
use crate::page_task_queue::RendererOwnerWakeSource;

#[tokio::test(flavor = "current_thread")]
async fn deprecated_storage_quota_uses_typed_async_misc_platform_tasks() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/deprecated-storage-quota").unwrap();
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        while owner_wake_rx.try_recv().is_ok() {}

        let initial = page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__legacyQuotaEvents = [];
  const callback = label => new Proxy(
    function() {
      "use strict";
      const values = Array.from(arguments);
      __legacyQuotaEvents.push(
        `${label}:${this === undefined}:${values.join(":")}`
      );
      Promise.resolve().then(() => {
        __legacyQuotaEvents.push(`microtask:${label}`);
      });
    },
    {
      apply(target, receiver, argumentsList) {
        __legacyQuotaEvents.push(
          `apply:${label}:${receiver === undefined}:${argumentsList.length}`
        );
        return Reflect.apply(target, receiver, argumentsList);
      }
    }
  );

  const conversion = [];
  for (const [label, invoke] of [
    ["query-missing", () =>
      navigator.webkitTemporaryStorage.queryUsageAndQuota()],
    ["query-object", () =>
      navigator.webkitTemporaryStorage.queryUsageAndQuota({})],
    ["query-error-object", () =>
      navigator.webkitTemporaryStorage.queryUsageAndQuota(() => {}, {})],
    ["request-error-object", () =>
      navigator.webkitPersistentStorage.requestQuota(1, null, {})],
    ["info-query-missing", () =>
      webkitStorageInfo.queryUsageAndQuota(TEMPORARY)],
    ["info-query-null", () =>
      webkitStorageInfo.queryUsageAndQuota(TEMPORARY, null)],
    ["info-query-object", () =>
      webkitStorageInfo.queryUsageAndQuota(TEMPORARY, {})],
    ["info-request-missing-quota", () =>
      webkitStorageInfo.requestQuota(PERSISTENT)]
  ]) {
    try {
      invoke();
      conversion.push(`${label}:ok`);
    } catch (error) {
      conversion.push(`${label}:${error.name}`);
    }
  }

  navigator.webkitTemporaryStorage.queryUsageAndQuota(callback("temporary"));
  navigator.webkitPersistentStorage.requestQuota(4096, callback("persistent"));
  webkitStorageInfo.queryUsageAndQuota(TEMPORARY, callback("info"));
  webkitStorageInfo.requestQuota(
    PERSISTENT,
    8192,
    callback("info-request")
  );
  navigator.webkitPersistentStorage.requestQuota(16);
  return JSON.stringify({ conversion, events: __legacyQuotaEvents });
})()
"#,
        )?;
        assert_eq!(
            initial,
            r#"{"conversion":["query-missing:TypeError","query-object:TypeError","query-error-object:TypeError","request-error-object:TypeError","info-query-missing:ok","info-query-null:ok","info-query-object:TypeError","info-request-missing-quota:TypeError"],"events":[]}"#,
            "all callback conversions must complete synchronously, but callback bodies must not"
        );

        let wakes = std::iter::from_fn(|| owner_wake_rx.try_recv().ok())
            .map(|wake| wake.source_for_test())
            .collect::<Vec<_>>();
        assert_eq!(
            wakes,
            vec![RendererOwnerWakeSource::MiscPlatformApiTask],
            "one nonempty MiscPlatformApi epoch must publish exactly one owner wake"
        );
        assert!(
            !page_vm.vm().has_ready_timeout(),
            "deprecated quota callbacks must not borrow the Page timer source"
        );

        for _ in 0..4 {
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::MiscPlatformApi,
                        &loader,
                    )
                    .await?,
                "each admitted callback must run through the production selected dispatcher"
            );
        }
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MiscPlatformApi,
                    &loader,
                )
                .await?,
            "failed conversions and a missing optional quota callback must publish no tasks"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__legacyQuotaEvents.join('|')")?,
            "apply:temporary:true:2|temporary:true:0:1073741824|microtask:temporary|\
             apply:persistent:true:1|persistent:true:4096|microtask:persistent|\
             apply:info:true:2|info:true:0:1073741824|microtask:info|\
             apply:info-request:true:1|info-request:true:8192|microtask:info-request",
            "FIFO, undefined receiver, callable proxies, result arguments, and task-end checkpoints must be preserved"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("typed deprecated-storage quota task test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn deprecated_storage_quota_separates_calling_target_and_callback_realms() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/deprecated-storage-quota-realms").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__legacyQuotaRealmEvents = [];
globalThis.__legacyQuotaParentErrors = 0;
onerror = () => {
  __legacyQuotaParentErrors += 1;
  return true;
};
const frame = document.createElement("iframe");
frame.id = "legacy-quota-callback-realm";
document.body.appendChild(frame);
"created"
"#,
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("legacy-quota-callback-realm")
            .expect("callback Realm fixture should expose its child handle");
        let child_context =
            materialize_only_child_realm_execution_context_through_page_turn_for_test(
                &mut page_vm,
                "legacy-quota-callback-realm",
            )?;
        page_vm.vm_mut().eval_in_child_default_context(
            child_context,
            r#"
onerror = (_message, _source, _line, _column, error) => {
  parent.__legacyQuotaRealmEvents.push(`child-error:${error && error.message}`);
  return true;
};
globalThis.__legacyQuotaChildCallback = function(usage, quota) {
  "use strict";
  parent.__legacyQuotaRealmEvents.push(
    `child:${globalThis === parent.document.getElementById("legacy-quota-callback-realm").contentWindow}:` +
    `${this === undefined}:${arguments.length}:${usage}:${quota}`
  );
  throw new Error("child-legacy-quota");
};
globalThis.__legacyQuotaRetiredCallback = function() {
  parent.__legacyQuotaRealmEvents.push("retired-callback-ran");
};
"installed"
"#,
        )?;

        page_vm.vm_mut().eval(
            r#"
navigator.webkitTemporaryStorage.queryUsageAndQuota(
  document.getElementById("legacy-quota-callback-realm")
    .contentWindow.__legacyQuotaChildCallback
);
"queued"
"#,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MiscPlatformApi,
                    &loader,
                )
                .await?
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__legacyQuotaRealmEvents)")?,
            r#"["child:true:true:2:0:1073741824","child-error:child-legacy-quota"]"#,
            "arguments, invocation, and exception reporting must use the callback Realm"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(__legacyQuotaParentErrors)")?,
            "0"
        );

        page_vm.vm_mut().eval(
            r#"
navigator.webkitTemporaryStorage.queryUsageAndQuota(
  document.getElementById("legacy-quota-callback-realm")
    .contentWindow.__legacyQuotaRetiredCallback
);
"queued"
"#,
        )?;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MiscPlatformApi,
            )
            .expect("current target callback task should be claimable");
        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(child_handle);
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__legacyQuotaRetiredCheckpoint = 0;
Promise.resolve().then(() => { __legacyQuotaRetiredCheckpoint += 1; });
"queued"
"#,
            )?;
        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(__legacyQuotaRetiredCheckpoint)",
            )?,
            "1",
            "a current calling target with a retired callback Realm still owns one checkpoint"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__legacyQuotaRealmEvents)")?,
            r#"["child:true:true:2:0:1073741824","child-error:child-legacy-quota"]"#,
            "retiring the callback Realm must suppress callback invocation"
        );

        page_vm.vm_mut().eval(
            r#"
const target = document.createElement("iframe");
target.id = "legacy-quota-calling-target";
document.body.appendChild(target);
globalThis.__legacyQuotaStaleEvents = [];
globalThis.__legacyQuotaParentCallback = () => {
  __legacyQuotaStaleEvents.push("callback");
};
"created"
"#,
        )?;
        let target_context =
            materialize_only_child_realm_execution_context_through_page_turn_for_test(
                &mut page_vm,
                "legacy-quota-calling-target",
            )?;
        page_vm.vm_mut().eval_in_child_default_context(
            target_context,
            r#"
navigator.webkitTemporaryStorage.queryUsageAndQuota(
  parent.__legacyQuotaParentCallback
);
"queued"
"#,
        )?;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MiscPlatformApi,
            )
            .expect("exact child-target callback task should be claimable");
        page_vm.vm_mut().eval(
            r#"document.getElementById("legacy-quota-calling-target").remove(); "removed""#,
        )?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__legacyQuotaStaleCheckpoint = 0;
Promise.resolve().then(() => { __legacyQuotaStaleCheckpoint += 1; });
"queued"
"#,
            )?;
        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(__legacyQuotaStaleCheckpoint)",
            )?,
            "0",
            "a stale calling target must not checkpoint an unrelated live Realm"
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "JSON.stringify(__legacyQuotaStaleEvents)",
            )?,
            "[]",
            "a stale calling target must discard its exact callback"
        );

        page_vm.vm_mut().eval(
            r#"
globalThis.__legacyQuotaReplacementEvents = [];
navigator.webkitTemporaryStorage.queryUsageAndQuota(() => {
  __legacyQuotaReplacementEvents.push("callback");
});
"queued"
"#,
        )?;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MiscPlatformApi,
            )
            .expect("root exact-Document callback task should be claimable");
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
globalThis.__legacyQuotaReplacementCheckpoint = 0;
Promise.resolve().then(() => {
  __legacyQuotaReplacementCheckpoint += 1;
});
"queued"
"#,
            )?;
        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(__legacyQuotaReplacementCheckpoint)",
            )?,
            "0",
            "an old exact-Document task must not checkpoint its replacement"
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "JSON.stringify(__legacyQuotaReplacementEvents)",
            )?,
            "[]",
            "document.open must retire the old exact-Document callback task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("deprecated storage quota exact-Realm test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn deprecated_storage_quota_reports_opaque_origin_errors_asynchronously() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("data:text/html,opaque-quota").unwrap();
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        while owner_wake_rx.try_recv().is_ok() {}

        assert_eq!(
            page_vm.vm_mut().eval(
                r#"
{
  [
    "DOMError" in globalThis,
    typeof DOMError,
  ].join("|")
}
"#,
            )?,
            "false|undefined",
            "the removed DOMError interface must not remain globally exposed"
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                r#"
globalThis.__legacyQuotaOpaqueEvents = [];
navigator.webkitTemporaryStorage.queryUsageAndQuota(
  () => { __legacyQuotaOpaqueEvents.push("unexpected-success"); },
  new Proxy(
    function(error) {
      "use strict";
      __legacyQuotaOpaqueEvents.push(
        `error:${this === undefined}:${arguments.length}:` +
        `${error.name}:${Object.prototype.toString.call(error)}:` +
        `${error instanceof DOMException}:${error.constructor.name}:` +
        `${error.message}:${"DOMError" in globalThis}:${typeof DOMError}`
      );
      Promise.resolve().then(() => {
        __legacyQuotaOpaqueEvents.push("microtask:error");
      });
    },
    {
      apply(target, receiver, argumentsList) {
        __legacyQuotaOpaqueEvents.push(
          `apply:${receiver === undefined}:${argumentsList.length}`
        );
        return Reflect.apply(target, receiver, argumentsList);
      }
    }
  )
);
JSON.stringify(__legacyQuotaOpaqueEvents)
"#,
            )?,
            "[]",
            "opaque-origin failure delivery must not nest inside the binding call"
        );
        assert_eq!(
            owner_wake_rx
                .try_recv()
                .expect("opaque error callback should wake the Page owner")
                .source_for_test(),
            RendererOwnerWakeSource::MiscPlatformApiTask
        );
        assert!(owner_wake_rx.try_recv().is_err());

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MiscPlatformApi,
                    &loader,
                )
                .await?
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__legacyQuotaOpaqueEvents.join('|')")?,
            "apply:true:1|error:true:1:NotSupportedError:[object DOMError]:false:DOMError:The implementation did not support the requested type of object or operation.:false:undefined|microtask:error"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("opaque deprecated storage quota error test should run");
}
