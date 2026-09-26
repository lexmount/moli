use super::http_fixture::StaticHttpServer;
use super::*;

#[tokio::test(flavor = "current_thread")]
async fn navigation_timing_inherits_native_resource_bindings_across_realms() {
    let server = StaticHttpServer::spawn_with_bodies(vec!["resource".to_owned()]).await;
    let resource_url = server.base_url().join("resource").unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader(server.base_url().as_str(), &loader);
    vm.eval(&format!(
        "globalThis.__navigationInheritanceResult = null; globalThis.__navigationResourceUrl = {};",
        serde_json::to_string(resource_url.as_str()).unwrap()
    ))
    .unwrap();
    let fixture = include_str!("../../../tests/fixtures/navigation-timing-inheritance.js");
    vm.eval(&format!(
        "({}).then(result => __navigationInheritanceResult = result, error => __navigationInheritanceResult = String(error));",
        fixture.trim().trim_end_matches(';')
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__navigationInheritanceResult !== null)",
        "true",
        "navigation timing native inheritance and receiver checks",
    )
    .await;
    let result = vm.eval("__navigationInheritanceResult").unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap_or_else(|error| {
        panic!("navigation inheritance probe returned {result:?}: {error}")
    });
    assert_eq!(result["total"], 1540);
    assert_eq!(result["failures"], serde_json::json!([]));
    assert_eq!(server.finish_targets().await, ["/resource"]);
}

#[test]
fn navigation_timing_json_reads_native_inherited_state_after_freezing_and_lifecycle() {
    let mut vm = new_storage_test_vm("https://navigation-timing-native.test/");
    vm.eval(
        r#"
        globalThis.navigationEntry = performance.getEntriesByType('navigation')[0];
        globalThis.nativeNavigationSnapshot = () =>
          PerformanceNavigationTiming.prototype.toJSON.call(navigationEntry);
        globalThis.initialNavigationSnapshot = nativeNavigationSnapshot();
        globalThis.navigationAuthorReads = 0;
        for (const name of Object.keys(initialNavigationSnapshot)) {
          Object.defineProperty(navigationEntry, name, {
            get() { navigationAuthorReads++; throw new Error('author getter called'); }
          });
        }
        Object.setPrototypeOf(navigationEntry, null);
        Object.freeze(navigationEntry);
        'ready';
        "#,
    )
    .unwrap();
    vm.dispatch_document_lifecycle_event("DOMContentLoaded")
        .unwrap();
    vm.dispatch_window_load_event().unwrap();
    let result = vm
        .eval(
            r#"
            (() => {
              const json = nativeNavigationSnapshot();
              const base = PerformanceEntry.prototype.toJSON.call(navigationEntry);
              const resource = PerformanceResourceTiming.prototype.toJSON.call(navigationEntry);
              const read = (prototype, key) =>
                Object.getOwnPropertyDescriptor(prototype, key).get.call(navigationEntry);
              return JSON.stringify({
                identity: ['name', 'entryType', 'startTime'].every(key =>
                  json[key] === initialNavigationSnapshot[key]
                  && base[key] === json[key] && resource[key] === json[key]
                  && read(PerformanceEntry.prototype, key) === json[key]),
                inheritedContentType: json.contentType === '' && resource.contentType === ''
                  && read(PerformanceResourceTiming.prototype, 'contentType') === '',
                // Web IDL's default serializer uses the interface declaring
                // the operation, including that interface's inherited members.
                interfaceKeySets: !Object.hasOwn(base, 'initiatorType')
                  && !Object.hasOwn(base, 'loadEventEnd')
                  && resource.initiatorType === 'navigation'
                  && !Object.hasOwn(resource, 'loadEventEnd'),
                lifecycleUpdated: initialNavigationSnapshot.loadEventEnd === 0
                  && json.domInteractive > 0
                  && json.domContentLoadedEventStart >= json.domInteractive
                  && json.domContentLoadedEventEnd >= json.domContentLoadedEventStart
                  && json.domComplete >= json.domContentLoadedEventEnd
                  && json.loadEventStart >= json.domComplete
                  && json.loadEventEnd >= json.loadEventStart
                  && json.duration === json.loadEventEnd,
                snapshotsAgree: Object.keys(base).every(key => base[key] === json[key])
                  && Object.keys(resource).every(key => resource[key] === json[key]),
                authorReads: navigationAuthorReads,
                frozen: Object.isFrozen(navigationEntry)
              });
            })()
            "#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"{"identity":true,"inheritedContentType":true,"interfaceKeySets":true,"lifecycleUpdated":true,"snapshotsAgree":true,"authorReads":0,"frozen":true}"#
    );
}
