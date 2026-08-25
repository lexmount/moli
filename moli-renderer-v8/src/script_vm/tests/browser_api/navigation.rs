use super::*;

#[test]
fn form_target_blank_reloads_rel_opener_policy_for_each_submission() {
    for (rel, expected_exposes_opener) in [
        ("", false),
        ("opener", true),
        ("noopener", false),
        ("opener noopener", false),
        ("opener noreferrer", false),
    ] {
        let mut vm = new_storage_test_vm("https://example.com/page.html");
        vm.eval(&format!(
            r#"
(() => {{
  const html = document.createElement("html");
  const body = document.createElement("body");
  html.appendChild(body);
  document.appendChild(html);
  const form = document.createElement("form");
  form.action = "/submitted";
  form.target = "_BLANK";
  form.rel = {rel:?};
  body.appendChild(form);
  form.submit();
}})()
"#
        ))
        .expect("target=_blank form submission should evaluate");

        let activations = vm.take_pending_popup_activations();
        assert_eq!(
            activations.len(),
            1,
            "rel={rel:?} should produce one auxiliary browsing-context action"
        );
        let crate::RendererPopupActivationSource::Window { exposes_opener, .. } =
            activations[0].source()
        else {
            panic!("form submission must retain its exact Window source");
        };
        assert_eq!(
            exposes_opener, &expected_exposes_opener,
            "rel={rel:?} opener policy"
        );
        assert_eq!(
            activations[0].disposition(),
            crate::RendererPopupDisposition::Foreground,
            "target=_blank form submission should select its new surface"
        );
    }
}

#[tokio::test]
async fn hyperlink_target_blank_reloads_rel_opener_policy_for_each_activation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_broadcast_channel_page_test_vm_with_loader("https://example.com/page.html", &loader);

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__hyperlinkPopupResults = [];
  globalThis.__hyperlinkPopupChannel = new BroadcastChannel("hyperlink-rel-policy");
  __hyperlinkPopupChannel.onmessage = event => __hyperlinkPopupResults.push(event.data);
  globalThis.__hyperlinkPopupUrl = label => URL.createObjectURL(new Blob([`
    <!doctype html>
    <script>
      new BroadcastChannel("hyperlink-rel-policy").postMessage({
        label: ${JSON.stringify(label)},
        hasOpener: opener !== null,
        referrer: document.referrer
      });
      window.close();
    <\/script>
  `], { type: "text/html" }));
  const html = document.createElement("html");
  const body = document.createElement("body");
  html.appendChild(body);
  document.appendChild(html);
  globalThis.__hyperlink = document.createElement("a");
  __hyperlink.target = "_blank";
  __hyperlink.rel = "noopener";
  __hyperlink.href = __hyperlinkPopupUrl("anchor-noopener");
  body.appendChild(__hyperlink);
  __hyperlink.click();
  return String(__hyperlinkPopupResults.length);
})()
"#,
        )
        .expect("anchor noopener popup setup should evaluate");
    assert_eq!(setup, "0");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__hyperlinkPopupResults.length)",
        "1",
        "anchor noopener popup should load",
    )
    .await;

    vm.eval(
        r#"
__hyperlink.rel = "opener";
__hyperlink.href = __hyperlinkPopupUrl("anchor-opener");
__hyperlink.click();
"#,
    )
    .expect("anchor opener popup should schedule");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__hyperlinkPopupResults.length)",
        "2",
        "anchor opener popup should load",
    )
    .await;

    vm.eval(
        r#"
globalThis.__hyperlink = document.createElement("area");
__hyperlink.target = "_blank";
__hyperlink.rel = "noreferrer";
__hyperlink.href = __hyperlinkPopupUrl("area-noreferrer");
document.body.appendChild(__hyperlink);
__hyperlink.click();
"#,
    )
    .expect("area noreferrer popup should schedule");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__hyperlinkPopupResults.length)",
        "3",
        "area noreferrer popup should load",
    )
    .await;

    vm.eval(
        r#"
__hyperlink.rel = "opener";
__hyperlink.href = __hyperlinkPopupUrl("area-opener");
__hyperlink.click();
"#,
    )
    .expect("area opener popup should schedule");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__hyperlinkPopupResults.length)",
        "4",
        "area opener popup should load",
    )
    .await;

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__hyperlinkPopupResults)")
            .expect("hyperlink popup relation results should evaluate"),
        r#"[{"label":"anchor-noopener","hasOpener":false,"referrer":"https://example.com/page.html"},{"label":"anchor-opener","hasOpener":true,"referrer":"https://example.com/page.html"},{"label":"area-noreferrer","hasOpener":false,"referrer":""},{"label":"area-opener","hasOpener":true,"referrer":"https://example.com/page.html"}]"#
    );
}

#[test]
fn document_readiness_events_precede_domcontentloaded_and_window_load() {
    let mut vm = new_storage_test_vm("https://document-readiness-events.test/");

    vm.eval(
        r#"
        globalThis.__documentReadinessEvents = [];
        globalThis.__windowLoadCalled = false;
        document.addEventListener("readystatechange", () => {
          globalThis.__documentReadinessEvents.push(`readystatechange:${document.readyState}`);
          if (document.readyState === "complete") {
            globalThis.__documentReadinessEvents.push(
              `load-before-complete-listener:${globalThis.__windowLoadCalled}`
            );
            window.addEventListener("load", () => {
              globalThis.__documentReadinessEvents.push("late-load-listener");
            }, { once: true });
          }
        });
        document.addEventListener("DOMContentLoaded", () => {
          globalThis.__documentReadinessEvents.push(`DOMContentLoaded:${document.readyState}`);
        });
        window.addEventListener("load", () => {
          globalThis.__windowLoadCalled = true;
          globalThis.__documentReadinessEvents.push(`load:${document.readyState}`);
        }, { once: true });
        "installed";
        "#,
    )
    .expect("document readiness listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("readiness test requires a current document owner");
    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser completion should prepare the interactive transition");

    vm.execute_post_parse_lifecycle_work_best_effort(
        PostParseLifecycleWork::ApplyMainDocumentInteractive(interactive),
    )
    .expect("interactive lifecycle work should dispatch");
    vm.execute_post_parse_lifecycle_work_best_effort(
        PostParseLifecycleWork::DispatchDomContentLoaded { owner },
    )
    .expect("DOMContentLoaded lifecycle work should dispatch");
    vm.execute_post_parse_lifecycle_work_best_effort(PostParseLifecycleWork::DispatchWindowLoad {
        owner,
    })
    .expect("window load lifecycle work should dispatch");

    let events = vm
        .eval("JSON.stringify(globalThis.__documentReadinessEvents)")
        .expect("document readiness event order should evaluate");
    assert_eq!(
        events,
        r#"["readystatechange:interactive","DOMContentLoaded:interactive","readystatechange:complete","load-before-complete-listener:false","load:complete","late-load-listener"]"#
    );
}

#[test]
fn performance_navigation_timing_constructor_matches_navigation_entries() {
    let mut vm = new_storage_test_vm("https://performance-navigation-timing.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const navigation = performance.getEntriesByType("navigation")[0];
              const attributeNames = [
                "initiatorType",
                "nextHopProtocol",
                "workerStart",
                "redirectStart",
                "redirectEnd",
                "fetchStart",
                "domainLookupStart",
                "domainLookupEnd",
                "connectStart",
                "connectEnd",
                "secureConnectionStart",
                "requestStart",
                "responseStart",
                "responseEnd",
                "transferSize",
                "encodedBodySize",
                "decodedBodySize",
                "unloadEventStart",
                "unloadEventEnd",
                "domInteractive",
                "domContentLoadedEventStart",
                "domContentLoadedEventEnd",
                "domComplete",
                "loadEventStart",
                "loadEventEnd",
                "type",
                "redirectCount"
              ];
              const descriptorStable = name => {
                const descriptor =
                  Object.getOwnPropertyDescriptor(PerformanceNavigationTiming.prototype, name);
                return !!descriptor
                  && typeof descriptor.get === "function"
                  && descriptor.get.name === `get ${name}`
                  && descriptor.get.length === 0
                  && descriptor.set === undefined
                  && descriptor.enumerable === true
                  && descriptor.configurable === true
                  && !Object.prototype.hasOwnProperty.call(navigation, name);
              };
              let constructError = "";
              try {
                new PerformanceNavigationTiming();
              } catch (error) {
                constructError = error.name;
              }
              return JSON.stringify({
                ctor: typeof PerformanceNavigationTiming,
                name: navigation && navigation.name,
                entryType: navigation && navigation.entryType,
                isNavigationTiming: navigation instanceof PerformanceNavigationTiming,
                inheritsPerformanceEntry: PerformanceNavigationTiming.prototype instanceof PerformanceEntry,
                prototypeAttributeNames: Object.getOwnPropertyNames(PerformanceNavigationTiming.prototype)
                  .filter(name => attributeNames.includes(name))
                  .join(","),
                descriptorsStable: attributeNames.every(descriptorStable),
                constructError,
              });
            })()
            "#,
        )
        .expect("performance navigation timing probe should evaluate");

    assert_eq!(
        result,
        r#"{"ctor":"function","name":"https://performance-navigation-timing.test/","entryType":"navigation","isNavigationTiming":true,"inheritsPerformanceEntry":true,"prototypeAttributeNames":"initiatorType,nextHopProtocol,workerStart,redirectStart,redirectEnd,fetchStart,domainLookupStart,domainLookupEnd,connectStart,connectEnd,secureConnectionStart,requestStart,responseStart,responseEnd,transferSize,encodedBodySize,decodedBodySize,unloadEventStart,unloadEventEnd,domInteractive,domContentLoadedEventStart,domContentLoadedEventEnd,domComplete,loadEventStart,loadEventEnd,type,redirectCount","descriptorsStable":true,"constructError":"TypeError"}"#
    );
}

#[test]
fn performance_entries_hide_backing_slots_and_ignore_spoofing() {
    let mut vm = new_storage_test_vm("https://performance-entry-slots.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const internalNames = entry => Object.getOwnPropertyNames(entry)
                .filter(name => name.startsWith("__moliPerformance"))
                .sort();
              const stringify = value => value === undefined ? "undefined" : String(value);
              const accessorShape = (prototype, receiver, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return [
                  name,
                  !!descriptor,
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.get?.length,
                  typeof descriptor?.set,
                  descriptor?.enumerable,
                  descriptor?.configurable,
                  Object.prototype.hasOwnProperty.call(receiver, name)
                ].join(":");
              };
              const navigation = performance.getEntriesByType("navigation")[0];
              const mark = performance.mark("real-mark");
              const measure = performance.measure("real-measure", {
                start: 1,
                duration: 2,
                detail: { source: "real" }
              });
              const initialNavigationNames = internalNames(navigation);
              const initialMarkNames = internalNames(mark);
              const initialMeasureNames = internalNames(measure);
              Object.defineProperties(mark, {
                __moliPerformanceEntryName: { value: "spoof", configurable: true },
                __moliPerformanceEntryType: { value: "resource", configurable: true },
                __moliPerformanceEntryStartTime: { value: 99, configurable: true },
                __moliPerformanceEntryDuration: { value: 99, configurable: true },
                __moliPerformanceEntryDetail: { value: "spoof", configurable: true }
              });
              Object.defineProperties(navigation, {
                __moliPerformanceEntryDuration: { value: 99, configurable: true },
                __moliPerformanceNavigationTimingLoadEventEnd: { value: 99, configurable: true },
                __moliPerformanceNavigationTimingType: { value: "reload", configurable: true }
              });
              const entryNameGetter =
                Object.getOwnPropertyDescriptor(PerformanceEntry.prototype, "name").get;
              const navTypeGetter =
                Object.getOwnPropertyDescriptor(PerformanceNavigationTiming.prototype, "type").get;
              const navLoadGetter =
                Object.getOwnPropertyDescriptor(PerformanceNavigationTiming.prototype, "loadEventEnd").get;
              const resourceGetters = [
                "initiatorType",
                "nextHopProtocol",
                "workerStart",
                "redirectStart",
                "redirectEnd",
                "fetchStart",
                "domainLookupStart",
                "domainLookupEnd",
                "connectStart",
                "connectEnd",
                "secureConnectionStart",
                "requestStart",
                "responseStart",
                "responseEnd",
                "transferSize",
                "encodedBodySize",
                "decodedBodySize",
                "renderBlockingStatus",
                "responseStatus",
                "contentType"
              ].map((name) => Object.getOwnPropertyDescriptor(PerformanceResourceTiming.prototype, name).get);
              const fake = {
                __moliPerformanceEntryName: "fake",
                __moliPerformanceNavigationTimingType: "reload",
                __moliPerformanceNavigationTimingLoadEventEnd: 99
              };
              return JSON.stringify({
                initialNavigationNames,
                initialMarkNames,
                initialMeasureNames,
                markName: mark.name,
                markEntryType: mark.entryType,
                markStartSpoofIgnored: mark.startTime !== 99,
                markDuration: mark.duration,
                markDetailNull: mark.detail === null,
                measureDetail: measure.detail.source,
                entryDescriptors: [
                  "name",
                  "entryType",
                  "startTime",
                  "duration"
                ].map(name => accessorShape(PerformanceEntry.prototype, mark, name)),
                detailDescriptors: [
                  accessorShape(PerformanceMark.prototype, mark, "detail"),
                  accessorShape(PerformanceMeasure.prototype, measure, "detail")
                ],
                resourceDescriptors: [
                  "initiatorType",
                  "nextHopProtocol",
                  "workerStart",
                  "redirectStart",
                  "redirectEnd",
                  "fetchStart",
                  "domainLookupStart",
                  "domainLookupEnd",
                  "connectStart",
                  "connectEnd",
                  "secureConnectionStart",
                  "requestStart",
                  "responseStart",
                  "responseEnd",
                  "transferSize",
                  "encodedBodySize",
                  "decodedBodySize",
                  "renderBlockingStatus",
                  "responseStatus",
                  "contentType"
                ].map(name => accessorShape(PerformanceResourceTiming.prototype, navigation, name)),
                navigationType: navigation.type,
                navigationLoadEventEnd: navigation.loadEventEnd,
                navigationDuration: navigation.duration,
                byRealMark: performance.getEntriesByName("real-mark", "mark").length,
                bySpoofMark: performance.getEntriesByName("spoof", "resource").length,
                fakeName: String(entryNameGetter.call(fake)),
                fakeNavigationType: String(navTypeGetter.call(fake)),
                fakeLoadEventEnd: String(navLoadGetter.call(fake)),
                fakeResourceValues: resourceGetters.map(getter => getter.call({})).map(stringify).join("|")
              });
            })()
            "#,
        )
        .expect("performance entry slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"initialNavigationNames":[],"initialMarkNames":[],"initialMeasureNames":[],"markName":"real-mark","markEntryType":"mark","markStartSpoofIgnored":true,"markDuration":0,"markDetailNull":true,"measureDetail":"real","entryDescriptors":["name:true:function:get name:0:undefined:true:true:false","entryType:true:function:get entryType:0:undefined:true:true:false","startTime:true:function:get startTime:0:undefined:true:true:false","duration:true:function:get duration:0:undefined:true:true:false"],"detailDescriptors":["detail:true:function:get detail:0:undefined:true:true:false","detail:true:function:get detail:0:undefined:true:true:false"],"resourceDescriptors":["initiatorType:true:function:get initiatorType:0:undefined:true:true:false","nextHopProtocol:true:function:get nextHopProtocol:0:undefined:true:true:false","workerStart:true:function:get workerStart:0:undefined:true:true:false","redirectStart:true:function:get redirectStart:0:undefined:true:true:false","redirectEnd:true:function:get redirectEnd:0:undefined:true:true:false","fetchStart:true:function:get fetchStart:0:undefined:true:true:false","domainLookupStart:true:function:get domainLookupStart:0:undefined:true:true:false","domainLookupEnd:true:function:get domainLookupEnd:0:undefined:true:true:false","connectStart:true:function:get connectStart:0:undefined:true:true:false","connectEnd:true:function:get connectEnd:0:undefined:true:true:false","secureConnectionStart:true:function:get secureConnectionStart:0:undefined:true:true:false","requestStart:true:function:get requestStart:0:undefined:true:true:false","responseStart:true:function:get responseStart:0:undefined:true:true:false","responseEnd:true:function:get responseEnd:0:undefined:true:true:false","transferSize:true:function:get transferSize:0:undefined:true:true:false","encodedBodySize:true:function:get encodedBodySize:0:undefined:true:true:false","decodedBodySize:true:function:get decodedBodySize:0:undefined:true:true:false","renderBlockingStatus:true:function:get renderBlockingStatus:0:undefined:true:true:false","responseStatus:true:function:get responseStatus:0:undefined:true:true:false","contentType:true:function:get contentType:0:undefined:true:true:false"],"navigationType":"navigate","navigationLoadEventEnd":0,"navigationDuration":0,"byRealMark":1,"bySpoofMark":0,"fakeName":"undefined","fakeNavigationType":"undefined","fakeLoadEventEnd":"undefined","fakeResourceValues":"undefined|undefined|undefined|undefined|undefined|undefined|undefined|undefined|undefined|undefined|undefined|undefined|undefined|undefined|undefined|undefined|undefined|undefined|undefined|undefined"}"#
    );
}

#[test]
fn performance_entry_to_json_returns_native_base_snapshot() {
    let mut vm = new_storage_test_vm("https://performance-entry-json.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const mark = performance.mark("json-mark", { startTime: 12 });
              const measure = performance.measure("json-measure", {
                start: 3,
                duration: 4
              });
              Object.defineProperties(mark, {
                name: { value: "spoof-name", configurable: true },
                entryType: { value: "spoof-type", configurable: true },
                startTime: { value: 99, configurable: true },
                duration: { value: 99, configurable: true }
              });
              const descriptor =
                Object.getOwnPropertyDescriptor(PerformanceEntry.prototype, "toJSON");
              let fakeError = "";
              try {
                descriptor.value.call(Object.create(PerformanceEntry.prototype));
              } catch (error) {
                fakeError = error.name;
              }
              return JSON.stringify({
                descriptor: [
                  descriptor.value.name,
                  descriptor.value.length,
                  descriptor.enumerable,
                  descriptor.writable,
                  descriptor.configurable
                ].join(":"),
                inherited: PerformanceMark.prototype.toJSON === descriptor.value
                  && PerformanceMeasure.prototype.toJSON === descriptor.value
                  && !Object.prototype.hasOwnProperty.call(PerformanceMark.prototype, "toJSON")
                  && !Object.prototype.hasOwnProperty.call(PerformanceMeasure.prototype, "toJSON"),
                mark: mark.toJSON(),
                measure: measure.toJSON(),
                fakeError
              });
            })()
            "#,
        )
        .expect("PerformanceEntry toJSON probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptor":"toJSON:0:true:true:true","inherited":true,"mark":{"name":"json-mark","entryType":"mark","startTime":12,"duration":0},"measure":{"name":"json-measure","entryType":"measure","startTime":3,"duration":4},"fakeError":"TypeError"}"#
    );
}

#[test]
fn parser_script_network_results_populate_buffered_resource_timing_snapshots() {
    let document_url = Url::parse("https://resource-timing-script.test/").expect("document URL");
    let script_url = Url::parse("https://resource-timing-script.test/app.js").expect("script URL");
    let mut vm = new_storage_test_vm(document_url.as_str());
    let response = Ok(crate::types::NavigationResponse::from_head_and_text_body(
        moli_fetch::ResponseHead {
            final_url: script_url.clone(),
            status: 200,
            headers: vec![(
                "Content-Type".to_owned(),
                "Application/JavaScript; charset=utf-8".to_owned(),
            )],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
        "void 0;".to_owned(),
    ));

    vm.record_script_subresource_network_result(document_url, script_url, &response);

    let result = vm
        .eval(
            r#"
            (() => {
              const observer = new PerformanceObserver(() => {});
              observer.observe({ type: "resource", buffered: true });
              const records = observer.takeRecords();
              const entry = records[0];
              const json = entry.toJSON();
              const toJSONDescriptor = Object.getOwnPropertyDescriptor(
                PerformanceResourceTiming.prototype,
                "toJSON"
              );
              let fakeToJSONError = "";
              try {
                toJSONDescriptor.value.call(Object.create(PerformanceResourceTiming.prototype));
              } catch (error) {
                fakeToJSONError = error.name;
              }
              const expectedJsonKeys = [
                "name",
                "entryType",
                "startTime",
                "duration",
                "initiatorType",
                "nextHopProtocol",
                "workerStart",
                "redirectStart",
                "redirectEnd",
                "fetchStart",
                "domainLookupStart",
                "domainLookupEnd",
                "connectStart",
                "connectEnd",
                "secureConnectionStart",
                "requestStart",
                "responseStart",
                "responseEnd",
                "transferSize",
                "encodedBodySize",
                "decodedBodySize",
                "renderBlockingStatus",
                "responseStatus",
                "contentType"
              ];
              return JSON.stringify({
                length: records.length,
                name: entry.name,
                entryType: entry.entryType,
                initiatorType: entry.initiatorType,
                instance: entry instanceof PerformanceResourceTiming,
                responseStatus: entry.responseStatus,
                contentType: entry.contentType,
                renderBlockingStatusValid:
                  entry.renderBlockingStatus === "blocking"
                    || entry.renderBlockingStatus === "non-blocking",
                transferSizePositive: entry.transferSize > entry.encodedBodySize,
                encodedBodySize: entry.encodedBodySize,
                decodedBodySize: entry.decodedBodySize,
                jsonOwnKeys: expectedJsonKeys.every(key =>
                  Object.prototype.hasOwnProperty.call(json, key)),
                jsonMatchesEntry: expectedJsonKeys.every(key => json[key] === entry[key]),
                toJSONDescriptor: [
                  toJSONDescriptor.value.name,
                  toJSONDescriptor.value.length,
                  toJSONDescriptor.enumerable,
                  toJSONDescriptor.writable,
                  toJSONDescriptor.configurable
                ].join(":"),
                fakeToJSONError
              });
            })()
            "#,
        )
        .expect("script resource timing snapshot probe should evaluate");

    assert_eq!(
        result,
        r#"{"length":1,"name":"https://resource-timing-script.test/app.js","entryType":"resource","initiatorType":"script","instance":true,"responseStatus":200,"contentType":"application/javascript","renderBlockingStatusValid":true,"transferSizePositive":true,"encodedBodySize":7,"decodedBodySize":7,"jsonOwnKeys":true,"jsonMatchesEntry":true,"toJSONDescriptor":"toJSON:0:true:true:true","fakeToJSONError":"TypeError"}"#
    );
}

#[test]
fn performance_root_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://performance-root-slots.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const internalNames = object => Object.getOwnPropertyNames(object)
                .filter(name => name.startsWith("__moliPerformance"))
                .sort();
              const accessorStable = (prototype, receiver, name, enumerable) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return !!descriptor
                  && typeof descriptor.get === "function"
                  && descriptor.get.name === `get ${name}`
                  && descriptor.get.length === 0
                  && descriptor.set === undefined
                  && descriptor.enumerable === enumerable
                  && descriptor.configurable === true
                  && !Object.prototype.hasOwnProperty.call(receiver, name);
              };
              const timing = performance.timing;
              const navigation = performance.navigation;
              const eventCounts = performance.eventCounts;
              const timeOrigin = performance.timeOrigin;
              const initialPerformanceNames = internalNames(performance);
              const initialNavigationNames = internalNames(navigation);
              const initialEventCountsNames = internalNames(eventCounts);
              Object.defineProperties(performance, {
                __moliPerformanceTimeOrigin: { value: 1, configurable: true },
                __moliPerformanceEntries: { value: [], configurable: true },
                __moliPerformanceTiming: { value: { spoof: true }, configurable: true },
                __moliPerformanceNavigation: { value: { type: 1 }, configurable: true },
                __moliPerformanceEventCounts: { value: { get: () => 99 }, configurable: true }
              });
              Object.defineProperties(navigation, {
                __moliPerformanceNavigationType: { value: 1, configurable: true },
                __moliPerformanceNavigationRedirectCount: { value: 9, configurable: true }
              });
              Object.defineProperty(eventCounts, "__moliPerformanceEventCountsValues", {
                value: [99],
                configurable: true
              });
              const performancePrototype = Object.getPrototypeOf(performance);
              const navigationPrototype = Object.getPrototypeOf(navigation);
              const eventCountsPrototype = Object.getPrototypeOf(eventCounts);
              const timeOriginGetter =
                Object.getOwnPropertyDescriptor(performancePrototype, "timeOrigin").get;
              const timingGetter =
                Object.getOwnPropertyDescriptor(performancePrototype, "timing").get;
              const navigationTypeGetter =
                Object.getOwnPropertyDescriptor(navigationPrototype, "type").get;
              const fakePerformance = {
                __moliPerformanceTimeOrigin: 1,
                __moliPerformanceTiming: { spoof: true }
              };
              const fakeNavigation = {
                __moliPerformanceNavigationType: 1
              };
              const fakeEventCounts = {
                __moliPerformanceEventCountsValues: [99]
              };
              const firstEntry = eventCounts.entries().next().value;
              const json = performance.toJSON();
              return JSON.stringify({
                initialPerformanceNames,
                initialNavigationNames,
                initialEventCountsNames,
                timeOriginSpoofIgnored: performance.timeOrigin === timeOrigin,
                timingStable: performance.timing === timing,
                navigationStable: performance.navigation === navigation,
                eventCountsStable: performance.eventCounts === eventCounts,
                performanceDescriptorsStable: ["timeOrigin", "timing", "navigation", "eventCounts"]
                  .every(name => accessorStable(performancePrototype, performance, name, true)),
                entriesSpoofIgnored: performance.getEntriesByType("navigation").length,
                navigationType: navigation.type,
                navigationRedirectCount: navigation.redirectCount,
                navigationDescriptorsStable: ["type", "redirectCount"]
                  .every(name => accessorStable(navigationPrototype, navigation, name, true)),
                eventCountsClick: eventCounts.get("click"),
                eventCountsFirstValue: eventCounts.values().next().value,
                eventCountsFirstEntry: `${firstEntry[0]}:${firstEntry[1]}`,
                jsonTimeOriginStable: json.timeOrigin === timeOrigin,
                jsonNavigationType: json.navigation.type,
                fakeTimeOrigin: String(timeOriginGetter.call(fakePerformance)),
                fakeTiming: String(timingGetter.call(fakePerformance)),
                fakeNavigationType: String(navigationTypeGetter.call(fakeNavigation)),
                fakeEventCountsGet: String(eventCountsPrototype.get.call(fakeEventCounts, "click")),
                fakeEventCountsValue: String(eventCountsPrototype.values.call(fakeEventCounts).next().value)
              });
            })()
            "#,
        )
        .expect("performance root slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"initialPerformanceNames":[],"initialNavigationNames":[],"initialEventCountsNames":[],"timeOriginSpoofIgnored":true,"timingStable":true,"navigationStable":true,"eventCountsStable":true,"performanceDescriptorsStable":true,"entriesSpoofIgnored":1,"navigationType":0,"navigationRedirectCount":0,"navigationDescriptorsStable":true,"eventCountsClick":0,"eventCountsFirstValue":0,"eventCountsFirstEntry":"auxclick:0","jsonTimeOriginStable":true,"jsonNavigationType":0,"fakeTimeOrigin":"undefined","fakeTiming":"undefined","fakeNavigationType":"undefined","fakeEventCountsGet":"0","fakeEventCountsValue":"0"}"#
    );
}

#[test]
fn performance_and_event_counts_prototype_methods_are_declared_operations() {
    let mut vm = new_storage_test_vm("https://performance-declared-methods.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const descriptorSummary = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return [
                  name,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":");
              };
              const accessorSummary = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return [
                  name,
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.get?.length,
                  descriptor?.enumerable,
                  descriptor?.configurable,
                  descriptor?.set === undefined
                ].join(":");
              };
              const performanceDescriptors = [
                descriptorSummary(Performance.prototype, "now"),
                descriptorSummary(Performance.prototype, "toJSON"),
                descriptorSummary(Performance.prototype, "mark"),
                descriptorSummary(Performance.prototype, "clearMarks"),
                descriptorSummary(Performance.prototype, "measure"),
                descriptorSummary(Performance.prototype, "clearMeasures"),
                descriptorSummary(Performance.prototype, "getEntries"),
                descriptorSummary(Performance.prototype, "getEntriesByType"),
                descriptorSummary(Performance.prototype, "getEntriesByName")
              ];
              const eventCountsDescriptors = [
                descriptorSummary(EventCounts.prototype, "get"),
                descriptorSummary(EventCounts.prototype, "has"),
                descriptorSummary(EventCounts.prototype, "keys"),
                descriptorSummary(EventCounts.prototype, "values"),
                descriptorSummary(EventCounts.prototype, "entries"),
                descriptorSummary(EventCounts.prototype, "forEach")
              ];
              const iteratorDescriptor =
                Object.getOwnPropertyDescriptor(EventCounts.prototype, Symbol.iterator);
              const eventCounts = performance.eventCounts;
              const sizeDescriptor =
                Object.getOwnPropertyDescriptor(EventCounts.prototype, "size");
              eventCounts.size = 7;
              const seen = [];
              const context = { label: "ctx" };
              eventCounts.forEach(function(value, key, counts) {
                if (seen.length < 2) {
                  seen.push([this.label, key, value, counts === eventCounts].join(":"));
                }
              }, context);
              performance.mark("declared-mark");
              performance.measure("declared-measure", {
                start: 0,
                duration: 1,
                detail: { declared: true }
              });
              return JSON.stringify({
                performanceDescriptors,
                eventCountsDescriptors,
                iteratorAlias: iteratorDescriptor?.value === EventCounts.prototype.entries,
                iteratorDescriptor: [
                  iteratorDescriptor?.enumerable,
                  iteratorDescriptor?.writable,
                  iteratorDescriptor?.configurable
                ].join(":"),
                sizeDescriptor: accessorSummary(EventCounts.prototype, "size"),
                sizeGetterFake: sizeDescriptor.get.call({ spoofed: true }),
                performanceOwnMethods: Object.getOwnPropertyNames(performance)
                  .filter(name => name === "now" || name === "mark" || name === "getEntries")
                  .join(","),
                eventCountsOwnMethods: Object.getOwnPropertyNames(eventCounts)
                  .filter(name => name === "get" || name === "entries" || name === "forEach")
                  .join(","),
                eventCountsOwnSizeAfterSet: Object.prototype.hasOwnProperty.call(eventCounts, "size"),
                nowNumber: typeof performance.now() === "number",
                markCount: performance.getEntriesByName("declared-mark", "mark").length,
                measureDetail: performance.getEntriesByName("declared-measure", "measure")[0].detail.declared,
                typeCount: performance.getEntriesByType("measure").filter(entry => entry.name === "declared-measure").length,
                allCount: performance.getEntries().filter(entry => entry.name === "declared-measure").length,
                eventCountsSize: eventCounts.size,
                hasClick: eventCounts.has("click"),
                getClick: eventCounts.get("click"),
                firstKey: eventCounts.keys().next().value,
                firstValue: eventCounts.values().next().value,
                firstEntry: eventCounts[Symbol.iterator]().next().value.join(":"),
                forEachSeen: seen.join("|"),
                jsonHasTimeOrigin: typeof performance.toJSON().timeOrigin === "number"
              });
            })()
            "#,
        )
        .expect("performance declared prototype method probe should evaluate");

    assert_eq!(
        result,
        r#"{"performanceDescriptors":["now:function:now:0:true:true:true","toJSON:function:toJSON:0:true:true:true","mark:function:mark:1:true:true:true","clearMarks:function:clearMarks:0:true:true:true","measure:function:measure:1:true:true:true","clearMeasures:function:clearMeasures:0:true:true:true","getEntries:function:getEntries:0:true:true:true","getEntriesByType:function:getEntriesByType:1:true:true:true","getEntriesByName:function:getEntriesByName:1:true:true:true"],"eventCountsDescriptors":["get:function:get:1:true:true:true","has:function:has:1:true:true:true","keys:function:keys:0:true:true:true","values:function:values:0:true:true:true","entries:function:entries:0:true:true:true","forEach:function:forEach:1:true:true:true"],"iteratorAlias":true,"iteratorDescriptor":"false:true:true","sizeDescriptor":"size:function:get size:0:true:true:true","sizeGetterFake":36,"performanceOwnMethods":"","eventCountsOwnMethods":"","eventCountsOwnSizeAfterSet":false,"nowNumber":true,"markCount":1,"measureDetail":true,"typeCount":1,"allCount":1,"eventCountsSize":36,"hasClick":true,"getClick":0,"firstKey":"auxclick","firstValue":0,"firstEntry":"auxclick:0","forEachSeen":"ctx:auxclick:0:true|ctx:click:0:true","jsonHasTimeOrigin":true}"#
    );
}

#[test]
fn host_event_timestamp_uses_performance_private_time_origin() {
    let mut vm = new_storage_test_vm("https://performance-host-event-time.test/");

    vm.eval(
        r#"
        (() => {
          window.__hostEventTimestampProbe = null;
          document.addEventListener("DOMContentLoaded", event => {
            window.__hostEventTimestampProbe = {
              stamp: event.timeStamp,
              now: performance.now(),
              own: Object.prototype.hasOwnProperty.call(event, "timeStamp")
            };
          });
          Object.defineProperty(performance, "__moliPerformanceTimeOrigin", {
            value: 1,
            configurable: true
          });
          return "ready";
        })()
        "#,
    )
    .expect("host event timestamp setup should evaluate");

    vm.dispatch_document_lifecycle_event("DOMContentLoaded")
        .expect("DOMContentLoaded should dispatch with a host event timestamp");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = window.__hostEventTimestampProbe;
              return JSON.stringify({
                observed: !!probe,
                withinNowRange: probe && probe.stamp >= 0 && probe.stamp <= probe.now,
                safeResolution: probe && Math.round(probe.stamp * 1000) % 5 === 0,
                ownTimeStamp: probe && probe.own
              });
            })()
            "#,
        )
        .expect("host event timestamp probe should evaluate");

    assert_eq!(
        result,
        r#"{"observed":true,"withinNowRange":true,"safeResolution":true,"ownTimeStamp":false}"#
    );
}

#[test]
fn performance_to_json_returns_declared_snapshot_objects() {
    let mut vm = new_storage_test_vm("https://performance-json.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const methodSummary = (object, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(object, name);
                return [
                  name,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable,
                  Object.prototype.hasOwnProperty.call(object, name)
                ].join(":");
              };
              const json = performance.toJSON();
              const timeOriginDescriptor = Object.getOwnPropertyDescriptor(json, "timeOrigin");
              const timingDescriptor = Object.getOwnPropertyDescriptor(json, "timing");
              const navigationDescriptor = Object.getOwnPropertyDescriptor(json, "navigation");
              const timingJson = performance.timing.toJSON();
              const navigationJson = performance.navigation.toJSON();
              const navigationEntry = performance.getEntriesByType("navigation")[0];
              const navigationEntryJson = navigationEntry.toJSON();
              return JSON.stringify({
                timeOriginNumber: typeof json.timeOrigin === "number",
                timeOriginEnumerable: timeOriginDescriptor && timeOriginDescriptor.enumerable === true,
                timingEnumerable: timingDescriptor && timingDescriptor.enumerable === true,
                navigationEnumerable: navigationDescriptor && navigationDescriptor.enumerable === true,
                timingSnapshot: json.timing.navigationStart === timingJson.navigationStart,
                navigationSnapshot: json.navigation.type === navigationJson.type,
                navigationEntrySnapshot: navigationEntryJson.type === navigationEntry.type
                  && navigationEntryJson.loadEventEnd === navigationEntry.loadEventEnd,
                timingOwn: Object.prototype.hasOwnProperty.call(json.timing, "navigationStart"),
                navigationOwn: Object.prototype.hasOwnProperty.call(json.navigation, "redirectCount"),
                toJsonDescriptors: [
                  methodSummary(performance.timing, "toJSON"),
                  methodSummary(performance.navigation, "toJSON"),
                  methodSummary(navigationEntry, "toJSON")
                ]
              });
            })()
            "#,
        )
        .expect("performance toJSON snapshot probe should evaluate");

    assert_eq!(
        result,
        r#"{"timeOriginNumber":true,"timeOriginEnumerable":true,"timingEnumerable":true,"navigationEnumerable":true,"timingSnapshot":true,"navigationSnapshot":true,"navigationEntrySnapshot":true,"timingOwn":true,"navigationOwn":true,"toJsonDescriptors":["toJSON:function:toJSON:0:false:true:true:true","toJSON:function:toJSON:0:false:true:true:true","toJSON:function:toJSON:0:false:true:true:true"]}"#
    );
}

#[test]
fn performance_navigation_observer_supports_idle_gap_probe() {
    let mut vm = new_storage_test_vm("https://performance-navigation-idle-gap.test/");
    vm.dispatch_document_lifecycle_event("DOMContentLoaded")
        .expect("DOMContentLoaded should update navigation timing");
    vm.dispatch_window_load_event()
        .expect("load should update navigation timing");

    let result = vm
        .eval(
            r#"
            (() => {
              const observer = new PerformanceObserver(() => {});
              observer.observe({ type: "navigation", buffered: true });
              const records = observer.takeRecords();
              observer.disconnect();
              const idleEvents = records
                .filter((entry) => entry.duration !== 0)
                .map((entry) => ({ start: entry.startTime, end: entry.startTime + entry.duration }));
              function firstIdleGap(events, minIdleGap) {
                const points = [];
                for (const { start, end } of events) {
                  points.push({ t: start, delta: 1 }, { t: end, delta: -1 });
                }
                points.sort((left, right) => left.t === right.t ? left.delta - right.delta : left.t - right.t);
                let depth = 0;
                for (let index = 0; index < points.length; index++) {
                  depth += points[index].delta;
                  if (depth === 0) {
                    const start = points[index].t;
                    const next = points[index + 1];
                    if (!next || next.t - start >= minIdleGap) return start;
                  }
                }
                throw new Error("no idle gap found");
              }
              let idleGapFound = false;
              try {
                idleGapFound = Number.isFinite(firstIdleGap(idleEvents, 100));
              } catch (_) {}
              const navigation = records[0];
              return JSON.stringify({
                supported: PerformanceObserver.supportedEntryTypes.join(","),
                records: records.length,
                entryType: navigation && navigation.entryType,
                finiteTiming: navigation
                  && Number.isFinite(navigation.startTime)
                  && Number.isFinite(navigation.duration),
                nonZeroDuration: navigation && navigation.duration > 0,
                idleGapFound
              });
            })()
            "#,
        )
        .expect("performance navigation idle-gap probe should evaluate");

    assert_eq!(
        result,
        r#"{"supported":"mark,measure,navigation,resource","records":1,"entryType":"navigation","finiteTiming":true,"nonZeroDuration":true,"idleGapFound":true}"#
    );
}

#[test]
fn performance_navigation_timing_updates_at_load_event_end() {
    let mut vm = new_storage_test_vm("https://performance-navigation-load.test/");

    let before = vm
        .eval(
            r#"
            (() => {
              const navigation = performance.getEntriesByType("navigation")[0];
              return JSON.stringify({
                duration: navigation.duration,
                loadEventEnd: navigation.loadEventEnd,
                byName: performance.getEntriesByName(location.href, "navigation").length,
                hasAttributes: [
                  "initiatorType",
                  "nextHopProtocol",
                  "domContentLoadedEventEnd",
                  "loadEventEnd",
                  "redirectCount",
                  "type"
                ].every((name) => name in navigation)
              });
            })()
            "#,
        )
        .expect("initial navigation timing probe should evaluate");
    assert_eq!(
        before,
        r#"{"duration":0,"loadEventEnd":0,"byName":1,"hasAttributes":true}"#
    );

    vm.dispatch_document_lifecycle_event("DOMContentLoaded")
        .expect("DOMContentLoaded should update navigation timing");
    vm.dispatch_window_load_event()
        .expect("load should update navigation timing");

    let after = vm
        .eval(
            r#"
            (() => {
              const navigation = performance.getEntriesByType("navigation")[0];
              return JSON.stringify({
                durationMatchesLoadEnd: navigation.duration === navigation.loadEventEnd,
                durationPositive: navigation.duration > 0,
                legacyLoadMatches: Math.abs(
                  (performance.timing.loadEventEnd - performance.timing.navigationStart) -
                  navigation.loadEventEnd
                ) < 1,
                domOrder: navigation.domInteractive > 0
                  && navigation.domContentLoadedEventStart >= navigation.domInteractive
                  && navigation.domContentLoadedEventEnd >= navigation.domContentLoadedEventStart
                  && navigation.domComplete >= navigation.domContentLoadedEventEnd
                  && navigation.loadEventStart >= navigation.domComplete
                  && navigation.loadEventEnd >= navigation.loadEventStart,
              });
            })()
            "#,
        )
        .expect("completed navigation timing probe should evaluate");

    assert_eq!(
        after,
        r#"{"durationMatchesLoadEnd":true,"durationPositive":true,"legacyLoadMatches":true,"domOrder":true}"#
    );
}

#[test]
fn materialized_legacy_performance_timing_uses_integer_milliseconds() {
    const TIME_ORIGIN: f64 = 1_700_000_000_000.75;

    let mut vm = new_storage_test_vm("https://performance-legacy-integer-live.test/");
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let window = scope.get_current_context().global(scope);
        crate::context_bootstrap::bind_window_performance_seed(
            scope,
            window,
            "navigate",
            TIME_ORIGIN,
        )
    })
    .expect("fractional Performance time origin should bind");

    let before = vm
        .eval(
            r#"
            (() => {
              const timing = performance.timing;
              globalThis.__legacyTimingBeforeLifecycle = timing;
              return JSON.stringify({
                timeOrigin: performance.timeOrigin,
                navigationStart: timing.navigationStart,
                allInteger: Object.values(timing.toJSON()).every(Number.isInteger)
              });
            })()
            "#,
        )
        .expect("initial legacy PerformanceTiming probe should evaluate");
    assert_eq!(
        before,
        r#"{"timeOrigin":1700000000000.75,"navigationStart":1700000000000,"allInteger":true}"#
    );

    vm.dispatch_document_lifecycle_event("DOMContentLoaded")
        .expect("DOMContentLoaded should update materialized PerformanceTiming");
    vm.dispatch_window_load_event()
        .expect("load should update materialized PerformanceTiming");

    let after = vm
        .eval(
            r#"
            (() => {
              const timing = performance.timing;
              const navigation = performance.getEntriesByType("navigation")[0];
              const legacyLoadEnd = timing.loadEventEnd - timing.navigationStart;
              return JSON.stringify({
                stable: timing === globalThis.__legacyTimingBeforeLifecycle,
                allInteger: Object.values(timing.toJSON()).every(Number.isInteger),
                legacyLoadEndInteger: Number.isInteger(legacyLoadEnd),
                navigationHighResolution: Number.isFinite(navigation.loadEventEnd)
                  && navigation.loadEventEnd > 0,
                withinIntegerQuantization: Math.abs(
                  legacyLoadEnd - navigation.loadEventEnd
                ) < 1
              });
            })()
            "#,
        )
        .expect("completed materialized legacy PerformanceTiming probe should evaluate");
    assert_eq!(
        after,
        r#"{"stable":true,"allInteger":true,"legacyLoadEndInteger":true,"navigationHighResolution":true,"withinIntegerQuantization":true}"#
    );
}

#[test]
fn lazily_materialized_legacy_performance_timing_uses_integer_milliseconds() {
    const TIME_ORIGIN: f64 = 1_700_000_000_000.75;

    let mut vm = new_storage_test_vm("https://performance-legacy-integer-lazy.test/");
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let window = scope.get_current_context().global(scope);
        crate::context_bootstrap::bind_window_performance_seed(
            scope,
            window,
            "navigate",
            TIME_ORIGIN,
        )
    })
    .expect("fractional Performance time origin should bind");

    vm.dispatch_document_lifecycle_event("DOMContentLoaded")
        .expect("DOMContentLoaded should record pending PerformanceTiming state");
    vm.dispatch_window_load_event()
        .expect("load should record pending PerformanceTiming state");

    let result = vm
        .eval(
            r#"
            (() => {
              const timing = performance.timing;
              const navigation = performance.getEntriesByType("navigation")[0];
              const legacyLoadEnd = timing.loadEventEnd - timing.navigationStart;
              return JSON.stringify({
                timeOrigin: performance.timeOrigin,
                navigationStart: timing.navigationStart,
                allInteger: Object.values(timing.toJSON()).every(Number.isInteger),
                legacyLoadEndInteger: Number.isInteger(legacyLoadEnd),
                navigationHighResolution: Number.isFinite(navigation.loadEventEnd)
                  && navigation.loadEventEnd > 0,
                withinIntegerQuantization: Math.abs(
                  legacyLoadEnd - navigation.loadEventEnd
                ) < 1
              });
            })()
            "#,
        )
        .expect("lazy legacy PerformanceTiming probe should evaluate");
    assert_eq!(
        result,
        r#"{"timeOrigin":1700000000000.75,"navigationStart":1700000000000,"allInteger":true,"legacyLoadEndInteger":true,"navigationHighResolution":true,"withinIntegerQuantization":true}"#
    );
}

#[test]
fn performance_navigation_observer_receives_entry_at_load_event_end() {
    let mut vm = new_storage_test_vm("https://performance-navigation-observer.test/");

    let setup = vm
        .eval(
            r#"
            (() => {
              globalThis.__navigationObserverRecords = [];
              globalThis.__navigationObserver = new PerformanceObserver((list) => {
                globalThis.__navigationObserverListProbe = {
                  instance: list instanceof PerformanceObserverEntryList,
                  tag: Object.prototype.toString.call(list),
                  keys: Object.keys(list).join(","),
                  ownSlots: Object.getOwnPropertyNames(list)
                    .filter(name => name.startsWith("__moliPerformanceEntryList"))
                    .sort(),
                  all: list.getEntries().length,
                  byType: list.getEntriesByType("navigation").length,
                  byName: list.getEntriesByName(location.href, "navigation").length,
                  missing: list.getEntriesByType("mark").length
                };
                for (const entry of list.getEntries()) {
                  globalThis.__navigationObserverRecords.push({
                    entryType: entry.entryType,
                    durationPositive: entry.duration > 0
                  });
                }
              });
              globalThis.__navigationObserver.observe({ entryTypes: ["navigation"] });
              return globalThis.__navigationObserver.takeRecords().length;
            })()
            "#,
        )
        .expect("navigation observer setup should evaluate");
    assert_eq!(setup, "0");

    vm.dispatch_document_lifecycle_event("DOMContentLoaded")
        .expect("DOMContentLoaded should update navigation timing");
    vm.dispatch_window_load_event()
        .expect("load should notify navigation observer");

    let records = vm
        .eval(
            r#"
            (() => {
              const records = globalThis.__navigationObserverRecords;
              return JSON.stringify({
                length: records.length,
                entryType: records[0] && records[0].entryType,
                durationPositive: records[0] && records[0].durationPositive,
                list: globalThis.__navigationObserverListProbe
              });
            })()
            "#,
        )
        .expect("navigation observer records should evaluate");

    assert_eq!(
        records,
        r#"{"length":1,"entryType":"navigation","durationPositive":true,"list":{"instance":true,"tag":"[object PerformanceObserverEntryList]","keys":"","ownSlots":[],"all":1,"byType":1,"byName":1,"missing":0}}"#
    );
}

#[test]
fn history_navigation_arguments_use_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const probe = (callback) => {
                try {
                  const value = callback();
                  return value === undefined ? "undefined" : String(value);
                } catch (error) {
                  return error && error.name;
                }
              };
              return [
                probe(() => history.pushState()),
                probe(() => history.pushState({ ok: 1 }, Symbol("unused"), "#bad")),
                probe(() => history.pushState({ ok: 1 }, {
                  toString() {
                    throw new RangeError("unused");
                  }
                }, "#bad")),
                probe(() => history.pushState({ ok: 1 }, "", Symbol("url"))),
                probe(() => history.replaceState({ ok: 2 }, "", {
                  toString() {
                    throw new RangeError("url");
                  }
                })),
                probe(() => history.pushState({ ok: 3 }, "", "#three")),
                location.hash,
                history.state.ok,
                probe(() => navigation.navigate()),
                probe(() => navigation.navigate(Symbol("url"))),
                probe(() => navigation.navigate({
                  toString() {
                    throw new RangeError("nav-url");
                  }
                })),
                probe(() => navigation.traverseTo()),
                probe(() => navigation.traverseTo(Symbol("key"))),
                probe(() => navigation.traverseTo({
                  toString() {
                    throw new RangeError("nav-key");
                  }
                }))
              ].join("|");
            })()
            "##,
        )
        .expect("history/navigation WebIDL argument probe should evaluate");

    assert_eq!(
        result,
        "TypeError|TypeError|RangeError|TypeError|RangeError|undefined|#three|3|TypeError|TypeError|RangeError|TypeError|TypeError|RangeError"
    );
}

#[test]
fn history_state_preserves_structured_clone_values_not_representable_as_json() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const sourceBuffer = new Uint8Array([1, 2, 255]).buffer;
              const source = {
                map: new Map([["answer", 42]]),
                set: new Set(["alpha", "beta"]),
                date: new Date("2024-03-04T05:06:07.000Z"),
                buffer: sourceBuffer,
                bigint: 9007199254740993n
              };
              history.pushState(source, "", "#rich");
              source.map.set("later", 9);
              new Uint8Array(sourceBuffer)[0] = 99;
              const stored = history.state;
              return JSON.stringify({
                hash: location.hash,
                brands: [
                  stored.map instanceof Map,
                  stored.set instanceof Set,
                  stored.date instanceof Date,
                  stored.buffer instanceof ArrayBuffer
                ],
                identities: [stored !== source, stored.map !== source.map, stored.buffer !== sourceBuffer],
                map: Array.from(stored.map),
                set: Array.from(stored.set),
                date: stored.date.toISOString(),
                bytes: Array.from(new Uint8Array(stored.buffer)),
                bigint: String(stored.bigint)
              });
            })()
            "##,
        )
        .expect("rich structured-clone history state should evaluate");

    assert_eq!(
        result,
        r##"{"hash":"#rich","brands":[true,true,true,true],"identities":[true,true,true],"map":[["answer",42]],"set":["alpha","beta"],"date":"2024-03-04T05:06:07.000Z","bytes":[1,2,255],"bigint":"9007199254740993"}"##
    );
}
#[test]
fn navigation_navigate_file_url_rejects_without_pending_location_navigation() {
    let mut vm = new_storage_test_vm("https://file-navigation.test/page.html");

    let setup = vm
        .eval(
            r#"
globalThis.__fileNavigationRejectLog = [];
const result = navigation.navigate("file://");
result.committed.then(
  () => globalThis.__fileNavigationRejectLog.push("committed:fulfilled"),
  error => globalThis.__fileNavigationRejectLog.push(`committed:${error.name}`)
);
result.finished.then(
  () => globalThis.__fileNavigationRejectLog.push("finished:fulfilled"),
  error => globalThis.__fileNavigationRejectLog.push(`finished:${error.name}`)
);
undefined;
"#,
        )
        .expect("file navigation setup should evaluate");
    assert_eq!(setup, "undefined");

    let settled = vm
        .eval("globalThis.__fileNavigationRejectLog.join('|')")
        .expect("file navigation rejection log should evaluate");
    assert_eq!(settled, "committed:AbortError|finished:AbortError");
    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "file URL navigation rejection should not queue a pending location navigation"
    );
}

#[test]
fn navigation_navigate_seed_uses_current_document_referrer_policy() {
    let mut vm = new_storage_test_vm("https://navigation-policy.test/start.html");
    vm.set_response_referrer_policy(Some("no-referrer".to_owned()));

    let result = vm
        .eval(
            r#"
const result = navigation.navigate("/next.html");
result.committed.catch(() => {});
result.finished.catch(() => {});
"queued"
"#,
        )
        .expect("navigation policy setup should evaluate");
    assert_eq!(result, "queued");

    let pending = vm
        .take_pending_location_navigation_with_seed()
        .expect("navigation.navigate should queue pending location navigation");
    assert_eq!(
        pending.url.as_str(),
        "https://navigation-policy.test/next.html"
    );
    let seed = pending
        .entry_seed
        .expect("cross-document navigation should carry history entry seed");
    let current_entry = seed
        .entries
        .iter()
        .find(|entry| entry.url == "https://navigation-policy.test/start.html")
        .expect("current document entry should be serialized into navigation seed");
    assert_eq!(
        current_entry.referrer_policy.as_deref(),
        Some("no-referrer"),
        "navigation seed must read the current document policy container"
    );
    let activation_from = seed
        .activation
        .as_ref()
        .and_then(|activation| activation.from.as_ref())
        .expect("same-origin navigation activation should expose from entry");
    assert_eq!(
        activation_from.referrer_policy.as_deref(),
        Some("no-referrer"),
        "activation.from should preserve the current document policy snapshot"
    );
}

#[test]
fn cross_document_pending_navigation_slot_is_not_script_writable() {
    let mut vm = new_storage_test_vm("https://cross-document-pending-slot.test/start.html");

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__lmCrossDocumentPendingSlotLog = [];
  const first = navigation.navigate("/first.html");
  first.committed.then(
    () => __lmCrossDocumentPendingSlotLog.push("firstCommitted"),
    error => __lmCrossDocumentPendingSlotLog.push(`firstCommittedRejected:${error.name}`)
  );
  first.finished.then(
    () => __lmCrossDocumentPendingSlotLog.push("firstFinished"),
    error => __lmCrossDocumentPendingSlotLog.push(`firstFinishedRejected:${error.name}`)
  );
  const exposedBefore = "__lmNavigationActiveCrossDocumentPending" in navigation;
  navigation.__lmNavigationActiveCrossDocumentPending = null;
  const second = navigation.navigate("/second.html");
  second.committed.catch(error => __lmCrossDocumentPendingSlotLog.push(`secondCommittedRejected:${error.name}`));
  second.finished.catch(error => __lmCrossDocumentPendingSlotLog.push(`secondFinishedRejected:${error.name}`));
  return JSON.stringify({
    exposedBefore,
    ownCrossDocumentSlots: Object.getOwnPropertyNames(navigation)
      .filter(name => name.startsWith("__lmCrossDocumentPending"))
      .join(","),
    publicSpoof: Object.hasOwn(navigation, "__lmNavigationActiveCrossDocumentPending"),
    log: __lmCrossDocumentPendingSlotLog.join("|")
  });
})()
"#,
        )
        .expect("cross-document pending slot setup should evaluate");

    assert_eq!(
        setup,
        r#"{"exposedBefore":false,"ownCrossDocumentSlots":"","publicSpoof":true,"log":""}"#
    );

    let pending = vm
        .take_pending_location_navigation_with_seed()
        .expect("second cross-document navigation should remain pending");
    assert_eq!(
        pending.url.as_str(),
        "https://cross-document-pending-slot.test/second.html"
    );

    let settled = vm
        .eval("globalThis.__lmCrossDocumentPendingSlotLog.join('|')")
        .expect("cross-document pending slot log should evaluate");
    assert_eq!(
        settled,
        "firstCommittedRejected:AbortError|firstFinishedRejected:AbortError"
    );
}

#[test]
fn location_hash_empty_fragment_serializes_empty_and_clears_target() {
    let mut vm = new_parsed_test_vm(
        "https://location-empty-fragment.test/#target",
        r##"
        <!doctype html>
        <html>
          <body>
            <a id="clear" href="#"></a>
            <div id="target"></div>
          </body>
        </html>
        "##,
    );

    let result = vm
        .eval(
            r##"
(() => {
  const target = document.getElementById("target");
  const initial = [location.hash, target.matches(":target")].join("/");
  document.getElementById("clear").click();
  return [
    initial,
    location.href.endsWith("#"),
    location.hash,
    target.matches(":target"),
    document.querySelector(":target") === null
  ].join("|");
})()
"##,
        )
        .expect("empty fragment hash probe should evaluate");

    assert_eq!(result, "#target/true|true||false|true");
}

#[test]
fn same_document_navigation_fires_navigate_event_before_mutation() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const seen = {};
              let capturedDestination = null;
              navigation.onnavigate = e => {
                capturedDestination = e.destination;
                seen.type = e.type;
                seen.navigationType = e.navigationType;
                seen.cancelable = e.cancelable;
                seen.canIntercept = e.canIntercept;
                seen.hashChange = e.hashChange;
                seen.destinationHash = new URL(e.destination.url).hash;
                seen.destinationState = e.destination.getState().ok;
                seen.destinationStateCloned = e.destination.getState() !== e.destination.getState();
                seen.destinationOwnSlots = Object.getOwnPropertyNames(e.destination)
                  .filter(name => name.startsWith("__lmNavigationDestination"))
                  .sort();
                Object.defineProperties(e.destination, {
                  __lmNavigationDestinationState: { value: { ok: 99 }, configurable: true },
                  __lmNavigationDestinationEntry: { value: null, configurable: true }
                });
                seen.destinationStateAfterSpoof = e.destination.getState().ok;
                seen.syntheticIntercept = (() => {
                  try {
                    new NavigateEvent("navigate", {
                      destination: e.destination,
                      signal: new AbortController().signal
                    }).intercept();
                    return "no throw";
                  } catch (error) {
                    return `${error.name}:${error instanceof DOMException}:${error.code}`;
                  }
                })();
                e.preventDefault();
              };
              const startLength = history.length;
              history.pushState({ ok: 7 }, "", "#blocked");
              return JSON.stringify({
                seen,
                hash: location.hash,
                state: history.state,
                lengthUnchanged: history.length === startLength,
                noInitThrows: (() => {
                  try {
                    new NavigateEvent("navigate");
                    return false;
                  } catch (error) {
                    return error.name === "TypeError";
                  }
                })(),
                missingSignalThrows: (() => {
                  try {
                    new NavigateEvent("navigate", { destination: capturedDestination });
                    return false;
                  } catch (error) {
                    return error.name === "TypeError";
                  }
                })()
              });
            })()
            "##,
        )
        .expect("same-document navigate event probe should evaluate");

    assert_eq!(
        result,
        r##"{"seen":{"type":"navigate","navigationType":"push","cancelable":true,"canIntercept":true,"hashChange":false,"destinationHash":"#blocked","destinationState":7,"destinationStateCloned":true,"destinationOwnSlots":[],"destinationStateAfterSpoof":7,"syntheticIntercept":"SecurityError:true:18"},"hash":"","state":null,"lengthUnchanged":true,"noInitThrows":true,"missingSignalThrows":true}"##
    );
}
#[test]
fn canceled_post_form_navigation_aborts_signal_without_synthetic_timer() {
    let mut vm = new_storage_test_vm("https://example.com/form-page");

    let setup = vm
        .eval(
            r##"
            (() => {
              const root = document.body || document.documentElement || document.appendChild(document.createElement("html"));
              if (!document.body && root === document.documentElement) {
                root.appendChild(document.createElement("body"));
              }
              const host = document.body || root;
              const form = document.createElement("form");
              form.action = "/submitted";
              form.method = "post";
              const input = document.createElement("input");
              input.name = "q";
              input.value = "value";
              form.appendChild(input);
              host.appendChild(form);
              globalThis.__lmCanceledFormNavigationLog = [];
              navigation.onnavigate = event => {
                __lmCanceledFormNavigationLog.push([
                  "navigate",
                  event.navigationType,
                  event.cancelable,
                  event.signal.aborted,
                  location.href
                ].join(":"));
                event.signal.addEventListener("abort", () => {
                  __lmCanceledFormNavigationLog.push([
                    "abort",
                    event.signal.reason.name,
                    location.href
                  ].join(":"));
                });
                event.preventDefault();
              };
              navigation.onnavigateerror = event => {
                __lmCanceledFormNavigationLog.push([
                  "error",
                  event.error.name,
                  location.href
                ].join(":"));
              };
              form.requestSubmit();
              return __lmCanceledFormNavigationLog.join("|");
            })()
            "##,
        )
        .expect("canceled form navigation setup should evaluate");

    assert_eq!(
        setup,
        "navigate:replace:true:false:https://example.com/form-page|abort:AbortError:https://example.com/form-page|error:AbortError:https://example.com/form-page"
    );
    assert!(
        !vm.has_ready_timeout(),
        "canceling a POST form navigation on the current event loop must not create a timer task"
    );
}

#[test]
fn cross_document_unload_lifecycle_orders_pagehide_before_unload_without_timer() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let lifecycle = vm
        .eval(
            r##"
            (() => {
              const log = [];
              addEventListener("beforeunload", () => log.push("beforeunload"));
              addEventListener("pagehide", event => log.push(`pagehide:${event.persisted}`));
              addEventListener("unload", () => log.push("unload"));
              navigation.navigate("/next-document");
              return log.join("|");
            })()
            "##,
        )
        .expect("cross-document unload lifecycle should evaluate");

    assert_eq!(lifecycle, "beforeunload|pagehide:false|unload");
    assert!(
        !vm.has_ready_timeout(),
        "pagehide is part of the unload step and must not create an independent timer task"
    );
}
#[test]
fn navigate_event_intercept_option_stringification_preserves_thrown_exception() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const seen = [];
              navigation.onnavigate = e => {
                for (const key of ["focusReset", "scroll"]) {
                  try {
                    e.intercept({
                      [key]: {
                        toString() {
                          throw new RangeError(`${key}:sentinel`);
                        }
                      }
                    });
                    seen.push(`${key}:no throw`);
                  } catch (error) {
                    seen.push(`${key}:${error.name}:${error.message}`);
                  }
                }
                e.preventDefault();
              };
              history.pushState(null, "", "#stringify-options");
              return seen.join("|");
            })()
            "##,
        )
        .expect("NavigateEvent intercept option probe should evaluate");

    assert_eq!(
        result,
        "focusReset:RangeError:focusReset:sentinel|scroll:RangeError:scroll:sentinel"
    );
}
#[test]
fn navigate_event_constructor_reflects_required_members_and_defaults() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const probeThrow = callback => {
                try {
                  callback();
                  return "no throw";
                } catch (error) {
                  return error.name;
                }
              };
              let destination = null;
              navigation.onnavigate = e => destination = e.destination;
              history.pushState({ statevar: "state" }, "", "#destination");
              const signal = new AbortController().signal;
              const info = { some: "object" };
              const formData = new FormData();
              const sourceElement = document.createElement("a");
              const methodNames = ["intercept", "deferPageSwap", "scroll"];
              const methodShape = (event, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(event, name);
                const value = descriptor && descriptor.value;
                return [
                  name,
                  typeof value,
                  value && value.name,
                  value && value.length,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.writable,
                  descriptor && descriptor.configurable,
                  Object.prototype.hasOwnProperty.call(event, name)
                ].join(":");
              };
              const full = new NavigateEvent("navigate", {
                navigationType: "replace",
                destination,
                canIntercept: true,
                userInitiated: true,
                hashChange: true,
                signal,
                formData,
                downloadRequest: "download",
                info,
                hasUAVisualTransition: true,
                sourceElement
              });
              const defaults = new NavigateEvent("navigate", { destination, signal });
              return JSON.stringify({
                noDictionary: probeThrow(() => new NavigateEvent("navigate")),
                noDestination: probeThrow(() => new NavigateEvent("navigate", {
                  signal,
                  canIntercept: false,
                  userInitiated: false,
                  hashChange: false
                })),
                noSignal: probeThrow(() => new NavigateEvent("navigate", { destination })),
                full: [
                  full.navigationType,
                  full.destination === destination,
                  full.canIntercept,
                  full.userInitiated,
                  full.hashChange,
                  full.signal === signal,
                  full.formData === formData,
                  full.downloadRequest,
                  full.info === info,
                  full.hasUAVisualTransition,
                  full.sourceElement === sourceElement
                ].join("|"),
                defaults: [
                  defaults.navigationType,
                  defaults.canIntercept,
                  defaults.userInitiated,
                  defaults.hashChange,
                  defaults.formData === null,
                  defaults.downloadRequest === null,
                  defaults.info === undefined,
                  defaults.sourceElement === null,
                  defaults.hasUAVisualTransition
                ].join("|"),
                methodKeys: Object.keys(full)
                  .filter(name => methodNames.includes(name))
                  .join(","),
                methods: methodNames.map(name => methodShape(full, name)).join("|")
              });
            })()
            "##,
        )
        .expect("NavigateEvent constructor probe should evaluate");

    assert_eq!(
        result,
        r#"{"noDictionary":"TypeError","noDestination":"TypeError","noSignal":"TypeError","full":"replace|true|true|true|true|true|true|download|true|true|true","defaults":"push|false|false|false|true|true|true|true|false","methodKeys":"intercept,deferPageSwap,scroll","methods":"intercept:function:intercept:0:true:true:true:true|deferPageSwap:function:deferPageSwap:0:true:true:true:true|scroll:function:scroll:0:true:true:true:true"}"#
    );
}
#[test]
fn navigation_transition_constructor_surface_is_present_but_illegal() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r#"
            (() => {
              const probeThrow = callback => {
                try {
                  callback();
                  return "no throw";
                } catch (error) {
                  return error.name;
                }
              };
              return JSON.stringify({
                type: typeof NavigationTransition,
                prototypeObject: typeof NavigationTransition.prototype,
                hasCommittedPrototypeMember: "committed" in NavigationTransition.prototype,
                initialTransition: navigation.transition,
                construct: probeThrow(() => new NavigationTransition())
              });
            })()
            "#,
        )
        .expect("NavigationTransition constructor surface should evaluate");

    assert_eq!(
        result,
        r#"{"type":"function","prototypeObject":"object","hasCommittedPrototypeMember":true,"initialTransition":null,"construct":"TypeError"}"#
    );
}
#[test]
fn intercepted_same_document_navigation_exposes_transition() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const from = navigation.currentEntry;
              globalThis.__lmInterceptTransitionLog = [];
              const record = name => globalThis.__lmInterceptTransitionLog.push({
                name,
                transitionObject: navigation.transition !== null,
                transitionBrand: navigation.transition instanceof NavigationTransition,
                fromMatches: navigation.transition?.from === from,
                navigationType: navigation.transition?.navigationType ?? null
              });
              navigation.addEventListener("navigate", () => record("navigate"));
              navigation.addEventListener("currententrychange", () => record("currententrychange"));
              navigation.addEventListener("navigatesuccess", () => record("navigatesuccess"));
              navigation.onnavigate = event => event.intercept({
                handler() { record("handler"); }
              });
              const result = navigation.navigate("#one");
              return JSON.stringify({
                log: globalThis.__lmInterceptTransitionLog,
                transitionAfterNavigate: navigation.transition !== null,
                committedPromise: typeof result.committed.then === "function",
                finishedPromise: typeof result.finished.then === "function"
              });
            })()
            "##,
        )
        .expect("intercepted transition probe should evaluate");

    assert_eq!(
        result,
        r##"{"log":[{"name":"navigate","transitionObject":false,"transitionBrand":false,"fromMatches":false,"navigationType":null},{"name":"currententrychange","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"push"},{"name":"handler","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"push"}],"transitionAfterNavigate":true,"committedPromise":true,"finishedPromise":true}"##
    );
    let after_microtask = vm
        .eval(
            r##"JSON.stringify({
              log: globalThis.__lmInterceptTransitionLog,
              transitionAfterNavigate: navigation.transition
            })"##,
        )
        .expect("intercepted transition microtask probe should evaluate");
    assert_eq!(
        after_microtask,
        r##"{"log":[{"name":"navigate","transitionObject":false,"transitionBrand":false,"fromMatches":false,"navigationType":null},{"name":"currententrychange","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"push"},{"name":"handler","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"push"},{"name":"navigatesuccess","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"push"}],"transitionAfterNavigate":null}"##
    );
}

#[test]
fn navigation_transition_object_keeps_declared_brand_and_members() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const from = navigation.currentEntry;
              let observed = "";
              navigation.onnavigate = event => event.intercept({
                handler() {
                  const transition = navigation.transition;
                  observed = [
                    transition instanceof NavigationTransition,
                    Object.prototype.toString.call(transition),
                    Object.keys(transition).join(","),
                    Object.getOwnPropertyNames(transition)
                      .filter(name => name.startsWith("__lmNavigationTransition"))
                      .join(","),
                    Object.hasOwn(transition, "from"),
                    Object.getOwnPropertyDescriptor(transition, "from").enumerable,
                    Object.getOwnPropertyDescriptor(transition, Symbol.toStringTag).writable,
                    transition.from === from,
                    transition.to !== null && typeof transition.to === "object",
                    transition.navigationType,
                    transition.committed instanceof Promise,
                    transition.finished instanceof Promise
                  ].join("|");
                }
              });
              navigation.navigate("#one");
              return observed;
            })()
            "##,
        )
        .expect("declared NavigationTransition probe should evaluate");

    assert_eq!(
        result,
        "true|[object NavigationTransition]|||true|false|false|true|true|push|true|true"
    );
}

#[test]
fn precommit_transition_seed_is_not_script_writable() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const from = navigation.currentEntry;
              let observed = "";
              navigation.onnavigate = event => {
                event.__lmNavigateEventPrecommitTransitionFrom = {};
                event.__lmNavigateEventPrecommitTransitionDestination = null;
                event.__lmNavigateEventPrecommitTransitionType = "reload";
                event.intercept({
                  precommitHandler() {
                    observed = [
                      navigation.transition?.from === from,
                      navigation.transition?.navigationType,
                      navigation.transition?.finished instanceof Promise
                    ].join("|");
                  }
                });
              };
              navigation.navigate("#one");
              return observed;
            })()
            "##,
        )
        .expect("precommit transition private seed probe should evaluate");

    assert_eq!(result, "true|push|true");
}

#[test]
fn precommit_controller_methods_are_declared_and_slots_private() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const log = globalThis.__lmPrecommitControllerLog = [];
              let observed = "";
              navigation.onnavigate = event => {
                event.intercept({
                  precommitHandler(controller) {
                    const initialNames = Object.getOwnPropertyNames(controller).sort();
                    Object.defineProperties(controller, {
                      __lmPrecommitControllerEvent: { value: null, configurable: true },
                      __lmPrecommitControllerActive: { value: false, configurable: true }
                    });
                    const addHandler = controller.addHandler;
                    const redirect = controller.redirect;
                    const addDescriptor = Object.getOwnPropertyDescriptor(controller, "addHandler");
                    const redirectDescriptor = Object.getOwnPropertyDescriptor(controller, "redirect");
                    controller.addHandler(() => log.push("added"));
                    controller.redirect("#declared");
                    const ownNames = Object.getOwnPropertyNames(controller).sort();
                    observed = JSON.stringify({
                      initialNames,
                      initialInternalNames: initialNames.filter(name => name.startsWith("__lm")),
                      ownNames,
                      keys: Object.keys(controller).sort(),
                      spoofedInternalNames: ownNames.filter(name => name.startsWith("__lm")),
                      addHandlerName: addHandler.name,
                      addHandlerLength: addHandler.length,
                      addHandlerEnumerable: addDescriptor.enumerable,
                      redirectName: redirect.name,
                      redirectLength: redirect.length,
                      redirectEnumerable: redirectDescriptor.enumerable,
                      destinationHash: new URL(event.destination.url).hash
                    });
                  },
                  handler() {
                    log.push(`handler:${location.hash}`);
                  }
                });
              };
              navigation.navigate("#one");
              return `${observed}|${log.join(",")}|${location.hash}`;
            })()
            "##,
        )
        .expect("precommit controller declared method probe should evaluate");

    assert_eq!(
        result,
        r##"{"initialNames":["addHandler","redirect"],"initialInternalNames":[],"ownNames":["__lmPrecommitControllerActive","__lmPrecommitControllerEvent","addHandler","redirect"],"keys":["addHandler","redirect"],"spoofedInternalNames":["__lmPrecommitControllerActive","__lmPrecommitControllerEvent"],"addHandlerName":"addHandler","addHandlerLength":1,"addHandlerEnumerable":true,"redirectName":"redirect","redirectLength":1,"redirectEnumerable":true,"destinationHash":"#declared"}||"##
    );
    assert_eq!(
        vm.eval("`${__lmPrecommitControllerLog.join(',')}|${location.hash}`")
            .expect("precommit controller handlers should settle"),
        "handler:#declared,added|#declared"
    );
}

#[test]
fn navigation_precommit_runs_after_complete_dispatch_and_before_commit_handlers() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let during_navigation = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmNavigationCallbackOrder = [];
              navigation.addEventListener("navigate", event => {
                __lmNavigationCallbackOrder.push("listener-1-before");
                event.intercept({
                  precommitHandler() {
                    __lmNavigationCallbackOrder.push("precommit");
                  },
                  handler() {
                    __lmNavigationCallbackOrder.push("handler");
                  }
                });
                __lmNavigationCallbackOrder.push("listener-1-after");
              });
              navigation.addEventListener("navigate", () => {
                __lmNavigationCallbackOrder.push("listener-2");
              });
              const result = navigation.navigate("#callback-order");
              result.committed.then(() => {
                __lmNavigationCallbackOrder.push("committed");
              });
              result.finished.then(() => {
                __lmNavigationCallbackOrder.push("finished");
              });
              return __lmNavigationCallbackOrder.join("|");
            })()
            "##,
        )
        .expect("Navigation callback ordering probe should evaluate");

    assert_eq!(
        during_navigation,
        "listener-1-before|listener-1-after|listener-2|precommit"
    );
    assert_eq!(
        vm.eval("__lmNavigationCallbackOrder.join('|')")
            .expect("Navigation callback Promise boundaries should settle"),
        "listener-1-before|listener-1-after|listener-2|precommit|handler|committed|finished"
    );
}

#[test]
fn navigation_handlers_use_webidl_callback_realms_proxies_and_promise_errors() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        (() => {
          const root =
            document.documentElement ||
            document.appendChild(document.createElement("html"));
          const body =
            document.body ||
            root.appendChild(document.createElement("body"));
          const frame = document.createElement("iframe");
          frame.id = "navigation-callback-frame";
          body.appendChild(frame);
          return "created";
        })()
        "#,
    )
    .expect("Navigation callback child Realm should be created");
    materialize_single_child_default_realm_for_test(
        &mut vm,
        "Navigation callback child Realm should materialize",
    );

    let during_navigation = vm
        .eval(
            r##"
            (() => {
              const frame = document.getElementById("navigation-callback-frame");
              const child = frame.contentWindow;
              globalThis.__lmNavigationCallbackFrame = frame;
              globalThis.__lmNavigationCallbackFacts = [];
              globalThis.__lmNavigationCallbackProxyCalls = [];
              globalThis.__lmNavigationCallbackError = null;

              const added = child.Function(`
                return new Proxy(
                  function() {
                    "use strict";
                    parent.__lmNavigationCallbackFacts.push({
                      name: "added",
                      callbackRealm:
                        globalThis === parent.__lmNavigationCallbackFrame.contentWindow,
                      receiver: this === parent.__lmNavigationCallbackEvent,
                      argumentCount: arguments.length
                    });
                  },
                  {
                    apply(target, receiver, args) {
                      parent.__lmNavigationCallbackProxyCalls.push("added");
                      return Reflect.apply(target, receiver, args);
                    }
                  }
                );
              `)();
              const precommit = child.Function("added", `
                return new Proxy(
                  function(controller) {
                    "use strict";
                    parent.__lmNavigationCallbackFacts.push({
                      name: "precommit",
                      callbackRealm:
                        globalThis === parent.__lmNavigationCallbackFrame.contentWindow,
                      receiver: this === parent.__lmNavigationCallbackEvent,
                      argumentCount: arguments.length
                    });
                    controller.addHandler(added);
                  },
                  {
                    apply(target, receiver, args) {
                      parent.__lmNavigationCallbackProxyCalls.push("precommit");
                      return Reflect.apply(target, receiver, args);
                    }
                  }
                );
              `)(added);
              const handler = child.Function(`
                return new Proxy(
                  function() {
                    "use strict";
                    parent.__lmNavigationCallbackFacts.push({
                      name: "handler",
                      callbackRealm:
                        globalThis === parent.__lmNavigationCallbackFrame.contentWindow,
                      receiver: this === parent.__lmNavigationCallbackEvent,
                      argumentCount: arguments.length
                    });
                    throw new Error("navigation-callback-realm-error");
                  },
                  {
                    apply(target, receiver, args) {
                      parent.__lmNavigationCallbackProxyCalls.push("handler");
                      return Reflect.apply(target, receiver, args);
                    }
                  }
                );
              `)();

              navigation.onnavigate = event => {
                globalThis.__lmNavigationCallbackEvent = event;
                event.intercept({
                  precommitHandler: precommit,
                  handler
                });
              };
              navigation.navigate("#callback-realm").finished.catch(error => {
                globalThis.__lmNavigationCallbackError = {
                  callbackRealm:
                    error instanceof child.Error && !(error instanceof Error),
                  message: error.message
                };
              });
              return JSON.stringify({
                facts: __lmNavigationCallbackFacts,
                proxyCalls: __lmNavigationCallbackProxyCalls,
                error: __lmNavigationCallbackError
              });
            })()
            "##,
        )
        .expect("Navigation callback-function semantics should queue");

    assert_eq!(
        during_navigation,
        r#"{"facts":[{"name":"precommit","callbackRealm":true,"receiver":true,"argumentCount":1}],"proxyCalls":["precommit"],"error":null}"#
    );
    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
              facts: __lmNavigationCallbackFacts,
              proxyCalls: __lmNavigationCallbackProxyCalls,
              error: __lmNavigationCallbackError
            })"#
        )
        .expect("Navigation callback-function Promise rejection should settle"),
        r#"{"facts":[{"name":"precommit","callbackRealm":true,"receiver":true,"argumentCount":1},{"name":"handler","callbackRealm":true,"receiver":true,"argumentCount":0},{"name":"added","callbackRealm":true,"receiver":true,"argumentCount":0}],"proxyCalls":["precommit","handler","added"],"error":{"callbackRealm":true,"message":"navigation-callback-realm-error"}}"#
    );
}

#[test]
fn navigation_handlers_skip_a_retired_callback_window_without_aborting_navigation() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        (() => {
          const root =
            document.documentElement ||
            document.appendChild(document.createElement("html"));
          const body =
            document.body ||
            root.appendChild(document.createElement("body"));
          const frame = document.createElement("iframe");
          frame.id = "retired-navigation-callback-frame";
          body.appendChild(frame);
          return "created";
        })()
        "#,
    )
    .expect("retired Navigation callback child Realm should be created");
    materialize_single_child_default_realm_for_test(
        &mut vm,
        "retired Navigation callback child Realm should materialize",
    );

    let during_navigation = vm
        .eval(
            r##"
            (() => {
              const frame = document.getElementById("retired-navigation-callback-frame");
              const child = frame.contentWindow;
              globalThis.__lmRetiredNavigationCallbackRuns = [];
              globalThis.__lmRetiredNavigationSettlements = [];
              const precommit = child.Function(
                `parent.__lmRetiredNavigationCallbackRuns.push("precommit");`
              );
              const handler = child.Function(
                `parent.__lmRetiredNavigationCallbackRuns.push("handler");`
              );
              navigation.addEventListener("navigate", event => {
                event.intercept({ precommitHandler: precommit, handler });
              }, { once: true });
              navigation.addEventListener("navigate", () => {
                frame.remove();
              }, { once: true });
              const result = navigation.navigate("#retired-callback-realm");
              result.committed.then(
                () => __lmRetiredNavigationSettlements.push("committed"),
                error => __lmRetiredNavigationSettlements.push(`committed:${error.name}`)
              );
              result.finished.then(
                () => __lmRetiredNavigationSettlements.push("finished"),
                error => __lmRetiredNavigationSettlements.push(`finished:${error.name}`)
              );
              return JSON.stringify({
                runs: __lmRetiredNavigationCallbackRuns,
                settlements: __lmRetiredNavigationSettlements
              });
            })()
            "##,
        )
        .expect("retired Navigation callbacks should queue");

    assert_eq!(during_navigation, r#"{"runs":[],"settlements":[]}"#);
    assert_eq!(
        vm.eval(
            "JSON.stringify({ runs: __lmRetiredNavigationCallbackRuns, settlements: __lmRetiredNavigationSettlements })"
        )
        .expect("retired Navigation callbacks should settle without running"),
        r#"{"runs":[],"settlements":["committed","finished"]}"#
    );
}

#[test]
fn navigate_event_internal_flags_are_not_script_writable() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const destination = {
                url: location.href,
                key: "",
                id: "",
                index: 0,
                sameDocument: true,
                getState() { return null; }
              };
              const event = new NavigateEvent("navigate", {
                destination,
                signal: new AbortController().signal,
                canIntercept: true
              });
              const exposedBefore = "__lmNavigateEventSynthetic" in event;
              event.__lmNavigateEventSynthetic = false;
              try {
                event.intercept({ handler() {} });
                return "allowed";
              } catch (error) {
                return `${error.name}:${exposedBefore}`;
              }
            })()
            "##,
        )
        .expect("NavigateEvent private flag tamper probe should evaluate");

    assert_eq!(result, "SecurityError:false");
}
#[test]
fn navigate_event_dispatching_flag_is_not_script_writable_after_dispatch() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              let captured;
              navigation.onnavigate = event => { captured = event; };
              navigation.navigate("#one");
              const exposed = "__lmDispatching" in captured;
              captured.__lmDispatching = true;
              try {
                captured.intercept({ handler() {} });
                return "allowed";
              } catch (error) {
                return `${error.name}:${exposed}`;
              }
            })()
            "##,
        )
        .expect("NavigateEvent dispatching flag tamper probe should evaluate");

    assert_eq!(result, "InvalidStateError:false");
}
#[test]
fn navigation_scroll_and_focus_slots_are_not_script_writable() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const html = document.documentElement || document.appendChild(document.createElement("html"));
              const body = document.body || html.appendChild(document.createElement("body"));
              body.innerHTML = "<button id='before'>before</button><input id='after'><div id='target'></div>";
              before.focus();
              const log = [];
              navigation.onnavigate = event => {
                event.intercept({
                  handler() {
                    const exposed = [
                      "__lmNavigationFocusResetEpoch" in navigation,
                      "__lmNavigationActiveScrollEvent" in navigation,
                      "__lmNavigationScrollTargetHref" in navigation
                    ].join(",");
                    const forgedEpoch = navigation.__lmNavigationFocusResetEpoch;
                    after.focus();
                    navigation.__lmNavigationFocusResetEpoch = forgedEpoch;
                    navigation.__lmNavigationActiveScrollEvent = {};
                    navigation.__lmNavigationScrollTargetHref = "https://example.com/forged";
                    log.push(`${exposed}:${document.activeElement.id}`);
                  }
                });
              };
              navigation.navigate("#target");
              log.push(`after:${document.activeElement.id}`);
              return log.join("|");
            })()
            "##,
        )
        .expect("navigation private slot tamper probe should evaluate");

    assert_eq!(result, "false,false,false:after|after:after");
}
#[test]
fn same_document_replace_dispatches_currententrychange_before_dispose() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const log = [];
              const original = navigation.currentEntry;
              original.ondispose = () => {
                log.push(`dispose:${log.includes("currententrychange")}:${original.index}`);
              };
              navigation.oncurrententrychange = event => {
                log.push("currententrychange");
                log.push(`from:${event.from === original}:${event.from.index}:${event.navigationType}`);
              };
              navigation.navigate("#replace", { history: "replace" });
              return log.join("|");
            })()
            "##,
        )
        .expect("replace dispose ordering probe should evaluate");

    assert_eq!(
        result,
        "currententrychange|from:true:-1:replace|dispose:true:-1"
    );
}
#[tokio::test]
async fn reentrant_same_document_navigation_aborts_active_navigate_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmReentrantNavigationLog = [];
              let first = true;
              navigation.addEventListener("navigate", event => {
                globalThis.__lmReentrantNavigationLog.push(`navigate:${location.hash}`);
                event.signal.addEventListener("abort", () => {
                  globalThis.__lmReentrantNavigationLog.push(`abort:${event.signal.reason.name}:${location.hash}`);
                });
                event.intercept({
                  handler: () => new Promise(resolve => {
                    setTimeout(() => {
                      globalThis.__lmReentrantNavigationLog.push(`handler:${location.hash}`);
                      resolve();
                    }, 0);
                  })
                });
                if (first) {
                  first = false;
                  const second = navigation.navigate("#two");
                  second.committed.then(
                    () => globalThis.__lmReentrantNavigationLog.push(`secondCommitted:${location.hash}`),
                    error => globalThis.__lmReentrantNavigationLog.push(`secondCommittedRejected:${error.name}`)
                  );
                  second.finished.then(
                    () => globalThis.__lmReentrantNavigationLog.push(`secondFinished:${location.hash}`),
                    error => globalThis.__lmReentrantNavigationLog.push(`secondFinishedRejected:${error.name}`)
                  );
                }
              });
              const firstResult = navigation.navigate("#one");
              firstResult.committed.then(
                () => globalThis.__lmReentrantNavigationLog.push("firstCommitted"),
                error => globalThis.__lmReentrantNavigationLog.push(`firstCommittedRejected:${error.name}`)
              );
              firstResult.finished.then(
                () => globalThis.__lmReentrantNavigationLog.push("firstFinished"),
                error => globalThis.__lmReentrantNavigationLog.push(`firstFinishedRejected:${error.name}`)
              );
              return `${location.hash}:${globalThis.__lmReentrantNavigationLog.join("|")}`;
            })()
            "##,
        )
        .expect("reentrant navigation setup should evaluate");

    assert_eq!(setup, "#two:navigate:|abort:AbortError:|navigate:");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("reentrant navigation handler should drain");
    let settled = vm
        .eval("globalThis.__lmReentrantNavigationLog.join('|')")
        .expect("reentrant navigation log should evaluate");

    assert_eq!(
        settled,
        "navigate:|abort:AbortError:|navigate:|secondCommitted:#two|firstCommittedRejected:AbortError|firstFinishedRejected:AbortError|handler:#two|secondFinished:#two"
    );
}
#[tokio::test]
async fn active_navigate_event_slot_is_not_script_writable() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmActiveNavigateSlotLog = [];
              Object.defineProperty(Object.prototype, "__lmActiveNavigateEventEvent", {
                configurable: true,
                value: { cancelable: false }
              });
              Object.defineProperty(Object.prototype, "__lmActiveNavigateEventSignal", {
                configurable: true,
                value: { addEventListener() {
                  globalThis.__lmActiveNavigateSlotLog.push("prototype-signal");
                } }
              });
              Object.defineProperty(Object.prototype, "__lmActiveNavigateEventHref", {
                configurable: true,
                value: "#spoofed"
              });
              globalThis.__lmActiveNavigateSlotLog.push([
                Object.prototype.__lmActiveNavigateEventEvent.cancelable === false,
                typeof Object.prototype.__lmActiveNavigateEventSignal.addEventListener,
                Object.prototype.__lmActiveNavigateEventHref
              ].join(":"));
              let first = true;
              navigation.addEventListener("navigate", event => {
                globalThis.__lmActiveNavigateSlotLog.push(`navigate:${location.hash}`);
                event.signal.addEventListener("abort", () => {
                  globalThis.__lmActiveNavigateSlotLog.push(`abort:${event.signal.reason.name}:${location.hash}`);
                });
                event.intercept({
                  handler: () => new Promise(resolve => {
                    setTimeout(() => {
                      globalThis.__lmActiveNavigateSlotLog.push(`handler:${location.hash}`);
                      resolve();
                    }, 0);
                  })
                });
                if (first) {
                  first = false;
                  globalThis.__lmActiveNavigateSlotLog.push(
                    `exposed:${"__lmNavigationActiveNavigateEvent" in navigation}`
                  );
                  navigation.__lmNavigationActiveNavigateEvent = null;
                  const second = navigation.navigate("#two");
                  second.committed.then(
                    () => globalThis.__lmActiveNavigateSlotLog.push(`secondCommitted:${location.hash}`),
                    error => globalThis.__lmActiveNavigateSlotLog.push(`secondCommittedRejected:${error.name}`)
                  );
                  second.finished.then(
                    () => globalThis.__lmActiveNavigateSlotLog.push(`secondFinished:${location.hash}`),
                    error => globalThis.__lmActiveNavigateSlotLog.push(`secondFinishedRejected:${error.name}`)
                  );
                }
              });
              const firstResult = navigation.navigate("#one");
              firstResult.committed.then(
                () => globalThis.__lmActiveNavigateSlotLog.push("firstCommitted"),
                error => globalThis.__lmActiveNavigateSlotLog.push(`firstCommittedRejected:${error.name}`)
              );
              firstResult.finished.then(
                () => globalThis.__lmActiveNavigateSlotLog.push("firstFinished"),
                error => globalThis.__lmActiveNavigateSlotLog.push(`firstFinishedRejected:${error.name}`)
              );
              return `${location.hash}:${globalThis.__lmActiveNavigateSlotLog.join("|")}`;
            })()
            "##,
        )
        .expect("active navigate event private slot probe should evaluate");

    assert_eq!(
        setup,
        "#two:true:function:#spoofed|navigate:|exposed:false|abort:AbortError:|navigate:"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("active navigate event private slot timers should drain");
    let settled = vm
        .eval("globalThis.__lmActiveNavigateSlotLog.join('|')")
        .expect("active navigate event private slot log should evaluate");

    assert_eq!(
        settled,
        "true:function:#spoofed|navigate:|exposed:false|abort:AbortError:|navigate:|secondCommitted:#two|firstCommittedRejected:AbortError|firstFinishedRejected:AbortError|handler:#two|secondFinished:#two"
    );
}
#[tokio::test]
async fn pending_precommit_navigation_slot_is_not_script_writable() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmPendingPrecommitSlotLog = [];
              Object.defineProperties(Object.prototype, {
                __lmPrecommitCommitActive: { configurable: true, value: false },
                __lmPrecommitCommitCurrentHref: { configurable: true, value: "https://spoofed.invalid/current" },
                __lmPrecommitCommitEffectiveHref: { configurable: true, value: "https://spoofed.invalid/effective" },
                __lmPrecommitCommitKind: { configurable: true, value: "reload" },
                __lmPrecommitCommitSignal: {
                  configurable: true,
                  value: { addEventListener() {
                    globalThis.__lmPendingPrecommitSlotLog.push("prototype-signal");
                  } }
                }
              });
              globalThis.__lmPendingPrecommitSlotLog.push(
                `precommitSpoof:${Object.prototype.__lmPrecommitCommitActive}:${Object.prototype.__lmPrecommitCommitKind}`
              );
              let first = true;
              navigation.onnavigate = event => {
                globalThis.__lmPendingPrecommitSlotLog.push(`navigate:${location.hash}`);
                event.signal.addEventListener("abort", () => {
                  globalThis.__lmPendingPrecommitSlotLog.push(`abort:${event.signal.reason.name}:${location.hash}`);
                });
                event.intercept({
                  precommitHandler: () => {
                    globalThis.__lmPendingPrecommitSlotLog.push(`precommit:${location.hash}`);
                    if (first) {
                      return new Promise(resolve => {
                        setTimeout(() => {
                          globalThis.__lmPendingPrecommitSlotLog.push(`first-precommit-timeout:${location.hash}`);
                          resolve();
                        }, 0);
                      });
                    }
                    return undefined;
                  },
                  handler: () => {
                    globalThis.__lmPendingPrecommitSlotLog.push(`handler:${location.hash}`);
                  }
                });
                if (first) {
                  first = false;
                }
              };
              const firstResult = navigation.navigate("#one");
              globalThis.__lmPendingPrecommitSlotLog.push(
                `exposed:${"__lmNavigationPendingPrecommitCommit" in navigation}`
              );
              navigation.__lmNavigationPendingPrecommitCommit = null;
              const second = navigation.navigate("#two");
              second.committed.then(
                () => globalThis.__lmPendingPrecommitSlotLog.push(`secondCommitted:${location.hash}`),
                error => globalThis.__lmPendingPrecommitSlotLog.push(`secondCommittedRejected:${error.name}`)
              );
              second.finished.then(
                () => globalThis.__lmPendingPrecommitSlotLog.push(`secondFinished:${location.hash}`),
                error => globalThis.__lmPendingPrecommitSlotLog.push(`secondFinishedRejected:${error.name}`)
              );
              firstResult.committed.then(
                () => globalThis.__lmPendingPrecommitSlotLog.push("firstCommitted"),
                error => globalThis.__lmPendingPrecommitSlotLog.push(`firstCommittedRejected:${error.name}`)
              );
              firstResult.finished.then(
                () => globalThis.__lmPendingPrecommitSlotLog.push("firstFinished"),
                error => globalThis.__lmPendingPrecommitSlotLog.push(`firstFinishedRejected:${error.name}`)
              );
              return `${location.hash}:${globalThis.__lmPendingPrecommitSlotLog.join("|")}`;
            })()
            "##,
        )
        .expect("pending precommit private slot probe should evaluate");

    assert_eq!(
        setup,
        ":precommitSpoof:false:reload|navigate:|precommit:|exposed:false|abort:AbortError:|navigate:|precommit:"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("pending precommit private slot timers should drain");
    let settled = vm
        .eval("`${location.hash}:${globalThis.__lmPendingPrecommitSlotLog.join('|')}`")
        .expect("pending precommit private slot log should evaluate");

    assert_eq!(
        settled,
        "#two:precommitSpoof:false:reload|navigate:|precommit:|exposed:false|abort:AbortError:|navigate:|precommit:|handler:#two|firstCommittedRejected:AbortError|firstFinishedRejected:AbortError|secondCommitted:#two|secondFinished:#two"
    );
}
#[tokio::test]
async fn nested_same_document_navigation_marks_outer_event_canceled() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmNestedNavigationCancelLog = [];
              let secondResult;
              navigation.onnavigate = event => {
                globalThis.__lmNestedNavigationCancelLog.push(`navigate:${event.info}:${event.defaultPrevented}:${location.hash}`);
                if (event.info === 1) {
                  secondResult = navigation.navigate("#two", { info: 2, history: "push" });
                  globalThis.__lmNestedNavigationCancelLog.push(`outerCanceled:${event.defaultPrevented}`);
                }
              };
              const firstResult = navigation.navigate("#one", { info: 1, history: "push" });
              firstResult.committed.then(
                () => globalThis.__lmNestedNavigationCancelLog.push("firstCommitted"),
                error => globalThis.__lmNestedNavigationCancelLog.push(`firstCommittedRejected:${error.name}`)
              );
              firstResult.finished.then(
                () => globalThis.__lmNestedNavigationCancelLog.push("firstFinished"),
                error => globalThis.__lmNestedNavigationCancelLog.push(`firstFinishedRejected:${error.name}`)
              );
              secondResult.committed.then(
                entry => globalThis.__lmNestedNavigationCancelLog.push(`secondCommitted:${new URL(entry.url).hash}`),
                error => globalThis.__lmNestedNavigationCancelLog.push(`secondCommittedRejected:${error.name}`)
              );
              secondResult.finished.then(
                entry => globalThis.__lmNestedNavigationCancelLog.push(`secondFinished:${new URL(entry.url).hash}`),
                error => globalThis.__lmNestedNavigationCancelLog.push(`secondFinishedRejected:${error.name}`)
              );
              return `${location.hash}:${navigation.entries().map(entry => new URL(entry.url).hash).join(",")}:${globalThis.__lmNestedNavigationCancelLog.join("|")}`;
            })()
            "##,
        )
        .expect("nested navigation cancellation setup should evaluate");
    assert_eq!(
        setup,
        "#two:,#two:navigate:1:false:|navigate:2:false:|outerCanceled:true"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("nested navigation cancellation should drain");
    let settled = vm
        .eval("globalThis.__lmNestedNavigationCancelLog.join('|')")
        .expect("nested navigation cancellation log should evaluate");
    assert_eq!(
        settled,
        "navigate:1:false:|navigate:2:false:|outerCanceled:true|firstCommittedRejected:AbortError|firstFinishedRejected:AbortError|secondCommitted:#two|secondFinished:#two"
    );
}
#[tokio::test]
async fn interrupted_intercepted_same_document_navigation_rejects_first_finished() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmDoubleInterceptLog = [];
              const record = name => {
                globalThis.__lmDoubleInterceptLog.push(`${name}:${location.hash}:${navigation.transition ? navigation.transition.navigationType : "null"}`);
              };
              Object.defineProperties(Object.prototype, {
                __lmInterceptSettlementActive: { configurable: true, value: false },
                __lmInterceptSettlementFilename: { configurable: true, value: "spoof.js" },
                __lmInterceptSettlementSignal: {
                  configurable: true,
                  value: { addEventListener() { record("prototype-signal"); } }
                },
                __lmInterceptSettlementCommittedResolve: {
                  configurable: true,
                  value() { record("prototype-committed"); }
                },
                __lmInterceptSettlementResolve: {
                  configurable: true,
                  value() { record("prototype-resolve"); }
                },
                __lmInterceptSettlementReject: {
                  configurable: true,
                  value() { record("prototype-reject"); }
                },
                __lmInterceptSettlementValue: { configurable: true, value: "spoofed" }
              });
              record(`interceptSpoof:${Object.prototype.__lmInterceptSettlementActive}:${Object.prototype.__lmInterceptSettlementFilename}`);
              navigation.addEventListener("navigate", event => record("navigate"));
              navigation.addEventListener("currententrychange", event => record("currententrychange"));
              navigation.addEventListener("navigatesuccess", event => record("navigatesuccess"));
              navigation.addEventListener("navigateerror", event => record(`navigateerror:${event.error?.name}`));
              navigation.addEventListener("navigate", event => {
                event.signal.addEventListener("abort", () => record(`abort:${event.signal.reason?.name}`));
                event.intercept({
                  handler: () => new Promise(resolve => {
                    record("handler");
                    setTimeout(() => {
                      record("handler-timeout");
                      resolve();
                    }, 1);
                  })
                });
              });
              const first = navigation.navigate("#one");
              first.committed.then(() => record("committed1"), error => record(`committed1-rejected:${error.name}`));
              first.finished.then(() => record("finished1"), error => record(`finished1-rejected:${error.name}`));
              navigation.transition?.finished.then(() => record("transition1"), error => record(`transition1-rejected:${error.name}`));
              const second = navigation.navigate("#two");
              second.committed.then(() => record("committed2"), error => record(`committed2-rejected:${error.name}`));
              second.finished.then(() => record("finished2"), error => record(`finished2-rejected:${error.name}`));
              navigation.transition?.finished.then(() => record("transition2"), error => record(`transition2-rejected:${error.name}`));
              Promise.resolve().then(() => record("microtask"));
              return globalThis.__lmDoubleInterceptLog.join("|");
            })()
            "##,
        )
        .expect("double intercept setup should evaluate");
    assert_eq!(
        setup,
        "interceptSpoof:false:spoof.js::null|navigate::null|currententrychange:#one:push|handler:#one:push|abort:AbortError:#one:push|navigateerror:AbortError:#one:push|navigate:#one:null|currententrychange:#two:push|handler:#two:push"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("double intercept timers should drain");
    let settled = vm
        .eval("globalThis.__lmDoubleInterceptLog.join('|')")
        .expect("double intercept log should evaluate");
    assert_eq!(
        settled,
        "interceptSpoof:false:spoof.js::null|navigate::null|currententrychange:#one:push|handler:#one:push|abort:AbortError:#one:push|navigateerror:AbortError:#one:push|navigate:#one:null|currententrychange:#two:push|handler:#two:push|committed1:#two:push|finished1-rejected:AbortError:#two:push|transition1-rejected:AbortError:#two:push|committed2:#two:push|microtask:#two:push|handler-timeout:#two:push|handler-timeout:#two:push|navigatesuccess:#two:push|finished2:#two:null|transition2:#two:null"
    );
}
#[tokio::test]
async fn location_href_double_intercept_cancels_first_settlement() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/start", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmLocationDoubleLog = [];
              const record = name => {
                globalThis.__lmLocationDoubleLog.push(`${name}:${location.href}:${navigation.transition ? navigation.transition.navigationType : "null"}`);
              };
              Object.defineProperties(Object.prototype, {
                __lmLocationInterceptActive: { configurable: true, value: false },
                __lmLocationInterceptFilename: { configurable: true, value: "spoof-location.js" },
                __lmLocationInterceptSignal: {
                  configurable: true,
                  value: { addEventListener() { record("prototype-signal"); } }
                }
              });
              record(`locationInterceptSpoof:${Object.prototype.__lmLocationInterceptActive}:${Object.prototype.__lmLocationInterceptFilename}`);
              navigation.addEventListener("navigate", event => {
                record("navigate");
                event.signal.addEventListener("abort", () => record(`abort:${event.signal.reason?.name}`));
                event.intercept({ handler() {
                  record("handler");
                  return new Promise(resolve => setTimeout(() => {
                    record("handler-timeout");
                    resolve();
                  }, 1));
                }});
              });
              navigation.addEventListener("currententrychange", () => record("currententrychange"));
              navigation.addEventListener("navigateerror", event => {
                record(`navigateerror:${event.error?.name}`);
                navigation.transition?.finished.then(
                  () => record("transition-finished"),
                  error => record(`transition-rejected:${error.name}`)
                );
              });
              navigation.addEventListener("navigatesuccess", () => {
                record("navigatesuccess");
                navigation.transition?.finished.then(
                  () => record("transition-finished"),
                  error => record(`transition-rejected:${error.name}`)
                );
              });
              location.href = "/common/blank.html#1";
              location.href = "/common/blank.html#2";
              Promise.resolve().then(() => record("microtask"));
              return globalThis.__lmLocationDoubleLog.join("|");
            })()
            "##,
        )
        .expect("location double setup should evaluate");
    assert_eq!(
        setup,
        "locationInterceptSpoof:false:spoof-location.js:https://example.com/start:null|navigate:https://example.com/start:null|currententrychange:https://example.com/common/blank.html#1:push|handler:https://example.com/common/blank.html#1:push|abort:AbortError:https://example.com/common/blank.html#1:push|navigateerror:AbortError:https://example.com/common/blank.html#1:push|navigate:https://example.com/common/blank.html#1:null|currententrychange:https://example.com/common/blank.html#2:replace|handler:https://example.com/common/blank.html#2:replace"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("location double timers should drain");
    let settled = vm
        .eval("globalThis.__lmLocationDoubleLog.join('|')")
        .expect("location double log should evaluate");
    assert_eq!(
        settled,
        "locationInterceptSpoof:false:spoof-location.js:https://example.com/start:null|navigate:https://example.com/start:null|currententrychange:https://example.com/common/blank.html#1:push|handler:https://example.com/common/blank.html#1:push|abort:AbortError:https://example.com/common/blank.html#1:push|navigateerror:AbortError:https://example.com/common/blank.html#1:push|navigate:https://example.com/common/blank.html#1:null|currententrychange:https://example.com/common/blank.html#2:replace|handler:https://example.com/common/blank.html#2:replace|transition-rejected:AbortError:https://example.com/common/blank.html#2:replace|microtask:https://example.com/common/blank.html#2:replace|handler-timeout:https://example.com/common/blank.html#2:replace|handler-timeout:https://example.com/common/blank.html#2:replace|navigatesuccess:https://example.com/common/blank.html#2:replace|transition-finished:https://example.com/common/blank.html#2:null"
    );
}
#[test]
fn same_document_navigation_dispatches_navigatesuccess() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmSameDocumentSuccessLog = [];
              navigation.onnavigatesuccess = () => {
                globalThis.__lmSameDocumentSuccessLog.push(`success:${location.hash}`);
              };
              navigation.navigate("#one");
              return globalThis.__lmSameDocumentSuccessLog.join("|");
            })()
            "##,
        )
        .expect("same-document navigatesuccess probe should evaluate");

    assert_eq!(result, "");
    let after_microtask = vm
        .eval("globalThis.__lmSameDocumentSuccessLog.join('|')")
        .expect("same-document navigatesuccess log should evaluate after microtask");
    assert_eq!(after_microtask, "success:#one");
}
#[test]
fn same_document_navigation_precommit_redirect_updates_destination() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let before_commit = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmPrecommitRedirectLog = [];
              globalThis.__lmPrecommitRedirectStartLength = navigation.entries().length;
              navigation.onnavigate = event => {
                event.intercept({
                  precommitHandler: controller => {
                    __lmPrecommitRedirectLog.push(`before:${location.hash}:${event.info.flag}:${event.destination.getState().value}`);
                    controller.redirect("#redirected", {
                      history: "replace",
                      info: { flag: "redirected" },
                      state: { value: 2 }
                    });
                    __lmPrecommitRedirectLog.push(`after:${location.hash}:${new URL(event.destination.url).hash}:${event.info.flag}:${event.destination.getState().value}`);
                  }
                });
              };
              navigation.navigate("#push", {
                history: "push",
                info: { flag: "initial" },
                state: { value: 1 }
              });
              return `${__lmPrecommitRedirectLog.join("|")}|pending:${location.hash}`;
            })()
            "##,
        )
        .expect("precommit redirect probe should evaluate");

    assert_eq!(
        before_commit,
        "before::initial:1|after::#redirected:redirected:2|pending:"
    );
    assert_eq!(
        vm.eval(
            r##"`final:${location.hash}:${navigation.entries().length - __lmPrecommitRedirectStartLength}:${navigation.currentEntry.getState().value}`"##
        )
        .expect("precommit redirect should commit after its Promise boundary"),
        "final:#redirected:0:2"
    );
}
#[tokio::test]
async fn same_document_navigation_precommit_added_handler_delays_finished() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmPrecommitAddHandlerLog = [];
              navigation.onnavigate = event => {
                event.intercept({
                  precommitHandler: controller => {
                    controller.addHandler(() => new Promise(resolve => {
                      setTimeout(() => {
                        globalThis.__lmPrecommitAddHandlerLog.push("added");
                        resolve();
                      }, 0);
                    }));
                  },
                  handler: () => {
                    globalThis.__lmPrecommitAddHandlerLog.push("handler");
                  }
                });
              };
              navigation.navigate("#one").finished.then(() => {
                globalThis.__lmPrecommitAddHandlerLog.push("finished");
              });
              return `${location.hash}:${globalThis.__lmPrecommitAddHandlerLog.join("|")}`;
            })()
            "##,
        )
        .expect("precommit addHandler setup should evaluate");

    assert_eq!(setup, ":");
    assert_eq!(
        vm.eval("`${location.hash}:${globalThis.__lmPrecommitAddHandlerLog.join('|')}`")
            .expect("precommit completion should commit before the added timer"),
        "#one:handler"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("precommit added handler timer should drain");
    let after_timeout = vm
        .eval("globalThis.__lmPrecommitAddHandlerLog.join('|')")
        .expect("precommit addHandler log should evaluate");

    assert_eq!(after_timeout, "handler|added|finished");
}
#[tokio::test]
async fn window_stop_cancels_pending_precommit_before_commit() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmStopPrecommitLog = [];
              navigation.onnavigate = event => {
                event.signal.addEventListener("abort", () => {
                  globalThis.__lmStopPrecommitLog.push(`abort:${event.signal.reason.name}:${location.search}`);
                });
                event.intercept({
                  precommitHandler: () => new Promise(() => {})
                });
              };
              navigation.onnavigateerror = () => {
                globalThis.__lmStopPrecommitLog.push(`error:${location.search}`);
              };
              navigation.onnavigatesuccess = () => {
                globalThis.__lmStopPrecommitLog.push("success");
              };
              const result = navigation.navigate("?blocked");
              result.committed.then(
                () => globalThis.__lmStopPrecommitLog.push("committed"),
                error => globalThis.__lmStopPrecommitLog.push(`committed-rejected:${error.name}`)
              );
              result.finished.then(
                () => globalThis.__lmStopPrecommitLog.push("finished"),
                error => globalThis.__lmStopPrecommitLog.push(`finished-rejected:${error.name}`)
              );
              window.stop();
              return `${location.search}:${globalThis.__lmStopPrecommitLog.join("|")}`;
            })()
            "##,
        )
        .expect("window.stop pending precommit setup should evaluate");

    assert_eq!(setup, ":abort:AbortError:|error:");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("window.stop pending precommit should drain");
    let settled = vm
        .eval("globalThis.__lmStopPrecommitLog.join('|')")
        .expect("window.stop pending precommit log should evaluate");
    assert_eq!(
        settled,
        "abort:AbortError:|error:|committed-rejected:AbortError|finished-rejected:AbortError"
    );
}
#[tokio::test]
async fn same_document_navigation_async_precommit_waits_to_commit() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmAsyncPrecommitLog = [];
              navigation.onnavigate = event => {
                event.intercept({
                  precommitHandler: () => new Promise(resolve => {
                    setTimeout(() => {
                      globalThis.__lmAsyncPrecommitLog.push(`precommit:${location.hash}`);
                      resolve();
                    }, 0);
                  }),
                  handler: () => {
                    globalThis.__lmAsyncPrecommitLog.push(`handler:${location.hash}`);
                  }
                });
              };
              const result = navigation.navigate("#one");
              result.committed.then(() => globalThis.__lmAsyncPrecommitLog.push(`committed:${location.hash}`));
              result.finished.then(() => globalThis.__lmAsyncPrecommitLog.push(`finished:${location.hash}`));
              return `${location.hash}:${globalThis.__lmAsyncPrecommitLog.join("|")}`;
            })()
            "##,
        )
        .expect("async precommit setup should evaluate");

    assert_eq!(setup, ":");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("async precommit timer should drain");
    let after_timeout = vm
        .eval("`${location.hash}:${globalThis.__lmAsyncPrecommitLog.join('|')}`")
        .expect("async precommit log should evaluate");

    assert_eq!(
        after_timeout,
        "#one:precommit:|handler:#one|committed:#one|finished:#one"
    );
}
#[tokio::test]
async fn same_document_navigation_async_precommit_reject_blocks_commit() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmAsyncPrecommitRejectLog = [];
              navigation.onnavigate = event => {
                event.intercept({
                  precommitHandler: () => new Promise((_, reject) => {
                    setTimeout(() => reject(new Error("blocked")), 0);
                  }),
                  handler: () => {
                    globalThis.__lmAsyncPrecommitRejectLog.push("handler");
                  }
                });
              };
              navigation.onnavigateerror = () => {
                globalThis.__lmAsyncPrecommitRejectLog.push(`error:${location.hash}`);
              };
              const result = navigation.navigate("#one");
              result.committed.catch(error => globalThis.__lmAsyncPrecommitRejectLog.push(`committed:${error.message}:${location.hash}`));
              result.finished.catch(error => globalThis.__lmAsyncPrecommitRejectLog.push(`finished:${error.message}:${location.hash}`));
              return `${location.hash}:${globalThis.__lmAsyncPrecommitRejectLog.join("|")}`;
            })()
            "##,
        )
        .expect("async precommit rejection setup should evaluate");

    assert_eq!(setup, ":");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("async precommit rejection timer should drain");
    let after_timeout = vm
        .eval("`${location.hash}:${globalThis.__lmAsyncPrecommitRejectLog.join('|')}`")
        .expect("async precommit rejection log should evaluate");

    assert_eq!(
        after_timeout,
        ":error:|committed:blocked:|finished:blocked:"
    );
}
#[tokio::test]
async fn same_document_navigation_finished_resolves_after_microtask() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let before_microtask_checkpoint = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmNavigationFinishedOrder = [];
              navigation.onnavigatesuccess = () => {
                globalThis.__lmNavigationFinishedOrder.push(`success:${location.hash}`);
              };
              const result = navigation.navigate("#one");
              result.committed.then(() => globalThis.__lmNavigationFinishedOrder.push("committed"));
              result.finished.then(() => globalThis.__lmNavigationFinishedOrder.push("finished"));
              Promise.resolve().then(() => globalThis.__lmNavigationFinishedOrder.push("microtask"));
              return globalThis.__lmNavigationFinishedOrder.join("|");
            })()
            "##,
        )
        .expect("same-document navigation ordering setup should evaluate");

    assert_eq!(before_microtask_checkpoint, "");

    let after_microtasks = vm
        .eval("globalThis.__lmNavigationFinishedOrder.join('|')")
        .expect("same-document navigation ordering log should evaluate after microtasks");

    assert_eq!(
        after_microtasks,
        "success:#one|committed|microtask|finished"
    );
}

#[tokio::test]
async fn closed_navigation_source_rejects_task_finished_without_timer_fallback() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    vm.eval(
        r#"
globalThis.__lmClosedNavigationTaskRoute = [];
navigation.navigate("/next-document");
"queued"
"#,
    )
    .expect("cross-document navigation should run its unload lifecycle step");
    assert!(
        !vm.has_ready_timeout(),
        "cross-document unload lifecycle must not leave a PageTimer before route retirement"
    );

    drop(
        vm._page_task_residence_for_executor_test
            .take()
            .expect("Navigation API route-retirement fixture should own one production consumer"),
    );
    vm.eval(
        r##"
navigation.onnavigatesuccess = () => __lmClosedNavigationTaskRoute.push("success");
navigation.onnavigateerror = event => {
  __lmClosedNavigationTaskRoute.push("error:" + event.error.name);
};
const result = navigation.navigate("#replacement");
result.committed.then(
  () => __lmClosedNavigationTaskRoute.push("committed"),
  error => __lmClosedNavigationTaskRoute.push("committed-error:" + error.name),
);
result.finished.then(
  () => __lmClosedNavigationTaskRoute.push("finished"),
  error => __lmClosedNavigationTaskRoute.push("finished-error:" + error.name),
);
"queued"
"##,
    )
    .expect("closed Navigation API route should reject instead of falling back");

    assert_eq!(
        vm.eval("globalThis.__lmClosedNavigationTaskRoute.join('|')")
            .expect("closed Navigation API route settlement should be observable"),
        "error:AbortError|error:AbortError|committed|finished-error:AbortError",
        "the retired cross-document attempt and the rejected replacement attempt should each dispatch one navigateerror"
    );
    assert!(
        !vm.has_ready_timeout(),
        "a closed navigation-and-traversal source must not recreate the removed timer transport"
    );
}

#[test]
fn same_document_intercept_rejects_finished_and_dispatches_navigateerror() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let setup = vm
        .eval(
            r##"
            (() => {
              const log = [];
              const err = new Error("boom");
              navigation.onnavigatesuccess = () => log.push("success");
              navigation.onnavigateerror = event => {
                log.push("error:" + String(event.error === err));
              };
              navigation.onnavigate = event => {
                event.intercept({ handler: () => Promise.reject(err) });
              };
              const result = navigation.navigate("#one");
              result.committed.then(() => log.push("committed"));
              result.finished.then(
                () => log.push("finished:fulfilled"),
                error => log.push("finished:" + String(error === err))
              );
              Promise.resolve().then(() => log.push("microtask"));
              globalThis.__lmInterceptRejectLog = log;
              return log.join("|");
            })()
            "##,
        )
        .expect("intercept reject setup should evaluate");
    assert_eq!(setup, "");

    let after_microtasks = vm
        .eval("globalThis.__lmInterceptRejectLog.join('|')")
        .expect("intercept reject microtasks should evaluate");
    assert_eq!(
        after_microtasks,
        "error:true|committed|microtask|finished:true"
    );
}
#[test]
fn same_document_multiple_intercept_handlers_reject_if_any_handler_rejects() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let setup = vm
        .eval(
            r##"
            (() => {
              const log = [];
              const err = new TypeError("sentinel");
              navigation.onnavigatesuccess = () => log.push("success");
              navigation.onnavigateerror = event => {
                log.push("error:" + String(event.error === err));
              };
              navigation.onnavigate = event => {
                event.intercept();
                event.intercept({ handler: () => Promise.reject(err) });
                event.intercept({ handler: () => Promise.resolve("ignored") });
              };
              const result = navigation.navigate("#one");
              result.finished.then(
                () => log.push("finished:fulfilled"),
                error => log.push("finished:" + String(error === err))
              );
              globalThis.__lmMultipleInterceptRejectLog = log;
              return log.join("|");
            })()
            "##,
        )
        .expect("multiple intercept rejection setup should evaluate");
    assert_eq!(setup, "");

    let after_microtasks = vm
        .eval("globalThis.__lmMultipleInterceptRejectLog.join('|')")
        .expect("multiple intercept rejection microtasks should evaluate");
    assert_eq!(after_microtasks, "error:true|finished:true");
}
#[test]
fn navigation_reload_intercept_rejects_finished_and_dispatches_navigateerror() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let setup = vm
        .eval(
            r##"
            (() => {
              const log = [];
              const err = new Error("reload blocked");
              const from = navigation.currentEntry;
              navigation.oncurrententrychange = event => {
                log.push(`change:${event.navigationType}:${event.from === from}`);
              };
              navigation.onnavigatesuccess = () => log.push("success");
              navigation.onnavigateerror = event => {
                log.push("error:" + String(event.error === err));
              };
              navigation.onnavigate = event => {
                log.push(`navigate:${event.navigationType}`);
                event.intercept({ handler: () => Promise.reject(err) });
              };
              const result = navigation.reload();
              result.committed.then(
                value => log.push("committed:" + String(value === navigation.currentEntry)),
                () => log.push("committed:rejected")
              );
              result.finished.then(
                () => log.push("finished:fulfilled"),
                error => log.push("finished:" + String(error === err))
              );
              Promise.resolve().then(() => log.push("microtask"));
              globalThis.__lmReloadInterceptRejectLog = log;
              return log.join("|");
            })()
            "##,
        )
        .expect("reload intercept rejection setup should evaluate");
    assert_eq!(setup, "navigate:reload|change:reload:true");

    let after_microtasks = vm
        .eval("globalThis.__lmReloadInterceptRejectLog.join('|')")
        .expect("reload intercept rejection microtasks should evaluate");
    assert_eq!(
        after_microtasks,
        "navigate:reload|change:reload:true|error:true|committed:true|microtask|finished:true"
    );
}
#[test]
fn navigation_reload_intercept_exposes_transition_during_events() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const from = navigation.currentEntry;
              globalThis.__lmReloadTransitionLog = [];
              const record = name => globalThis.__lmReloadTransitionLog.push({
                name,
                transitionObject: navigation.transition !== null,
                transitionBrand: navigation.transition instanceof NavigationTransition,
                fromMatches: navigation.transition?.from === from,
                navigationType: navigation.transition?.navigationType ?? null
              });
              navigation.addEventListener("navigate", () => record("navigate"));
              navigation.addEventListener("currententrychange", () => record("currententrychange"));
              navigation.addEventListener("navigatesuccess", () => record("navigatesuccess"));
              navigation.onnavigate = event => event.intercept({
                handler() { record("handler"); }
              });
              const reloadResult = navigation.reload();
              return JSON.stringify({
                log: globalThis.__lmReloadTransitionLog,
                transitionAfterReload: navigation.transition !== null,
                committedPromise: typeof reloadResult.committed.then === "function",
                finishedPromise: typeof reloadResult.finished.then === "function"
              });
            })()
            "##,
        )
        .expect("reload intercepted transition probe should evaluate");

    assert_eq!(
        result,
        r##"{"log":[{"name":"navigate","transitionObject":false,"transitionBrand":false,"fromMatches":false,"navigationType":null},{"name":"currententrychange","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"reload"},{"name":"handler","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"reload"}],"transitionAfterReload":true,"committedPromise":true,"finishedPromise":true}"##
    );
    let after_microtask = vm
        .eval(
            r##"JSON.stringify({
              log: globalThis.__lmReloadTransitionLog,
              transitionAfterReload: navigation.transition
            })"##,
        )
        .expect("reload intercepted transition microtask probe should evaluate");
    assert_eq!(
        after_microtask,
        r##"{"log":[{"name":"navigate","transitionObject":false,"transitionBrand":false,"fromMatches":false,"navigationType":null},{"name":"currententrychange","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"reload"},{"name":"handler","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"reload"},{"name":"navigatesuccess","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"reload"}],"transitionAfterReload":null}"##
    );
}

#[test]
fn navigation_state_clone_rejects_dom_nodes_before_document_state() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    vm.eval(
        r##"
        (() => {
          globalThis.__lmNavigationStateCloneLog = [];
          const record = (label, result) => {
            result.committed.then(
              () => __lmNavigationStateCloneLog.push(`${label}:committed:fulfilled`),
              error => __lmNavigationStateCloneLog.push(`${label}:committed:${error && error.name}`)
            );
            result.finished.then(
              () => __lmNavigationStateCloneLog.push(`${label}:finished:fulfilled`),
              error => __lmNavigationStateCloneLog.push(`${label}:finished:${error && error.name}`)
            );
          };
          const stateNode = document.createElement("div");

          record("active-navigate", navigation.navigate("?node-state", { state: stateNode }));
          record("active-reload", navigation.reload({ state: stateNode }));

          const host = document.documentElement || document.appendChild(document.createElement("html"));
          const frame = document.createElement("iframe");
          host.appendChild(frame);
          const child = frame.contentWindow;
          frame.remove();
          record("detached-navigate", child.navigation.navigate("/next", { state: stateNode }));
          record("detached-reload", child.navigation.reload({ state: stateNode }));
          return "queued";
        })()
        "##,
    )
    .expect("navigation state clone rejection setup should evaluate");

    let result = vm
        .eval("globalThis.__lmNavigationStateCloneLog.join('|')")
        .expect("navigation state clone rejection log should evaluate");

    assert_eq!(
        result,
        "active-navigate:committed:DataCloneError|active-navigate:finished:DataCloneError|active-reload:committed:DataCloneError|active-reload:finished:DataCloneError|detached-navigate:committed:DataCloneError|detached-navigate:finished:DataCloneError|detached-reload:committed:DataCloneError|detached-reload:finished:DataCloneError"
    );
}

#[tokio::test]
async fn navigation_reload_async_precommit_reject_blocks_commit() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmReloadPrecommitRejectLog = [];
              navigation.onnavigate = event => {
                event.intercept({
                  precommitHandler: () => new Promise((_, reject) => {
                    setTimeout(() => reject(new Error("reload blocked")), 0);
                  }),
                  handler: () => {
                    globalThis.__lmReloadPrecommitRejectLog.push("handler");
                  }
                });
              };
              navigation.oncurrententrychange = () => {
                globalThis.__lmReloadPrecommitRejectLog.push("change");
              };
              navigation.onnavigateerror = () => {
                globalThis.__lmReloadPrecommitRejectLog.push("error");
              };
              const result = navigation.reload();
              result.committed.then(
                () => globalThis.__lmReloadPrecommitRejectLog.push("committed:fulfilled"),
                error => globalThis.__lmReloadPrecommitRejectLog.push(`committed:${error.message}`)
              );
              result.finished.then(
                () => globalThis.__lmReloadPrecommitRejectLog.push("finished:fulfilled"),
                error => globalThis.__lmReloadPrecommitRejectLog.push(`finished:${error.message}`)
              );
              return globalThis.__lmReloadPrecommitRejectLog.join("|");
            })()
            "##,
        )
        .expect("reload async precommit rejection setup should evaluate");

    assert_eq!(setup, "");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("reload async precommit rejection timer should drain");
    let after_timeout = vm
        .eval("globalThis.__lmReloadPrecommitRejectLog.join('|')")
        .expect("reload async precommit rejection log should evaluate");

    assert_eq!(
        after_timeout,
        "error|committed:reload blocked|finished:reload blocked"
    );
}
#[test]
fn same_document_location_intercept_reject_dispatches_navigateerror() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let setup = vm
        .eval(
            r##"
            (() => {
              const log = [];
              const err = new Error("boom");
              navigation.onnavigatesuccess = () => log.push("success");
              navigation.onnavigateerror = event => {
                log.push("error:" + String(event.error === err) + ":" + location.hash);
              };
              navigation.onnavigate = event => {
                event.intercept({ handler: () => Promise.reject(err) });
              };
              location.href = "#one";
              Promise.resolve().then(() => log.push("microtask"));
              globalThis.__lmLocationInterceptRejectLog = log;
              return log.join("|");
            })()
            "##,
        )
        .expect("location intercept reject setup should evaluate");
    assert_eq!(setup, "");

    let after_microtasks = vm
        .eval("globalThis.__lmLocationInterceptRejectLog.join('|')")
        .expect("location intercept reject microtasks should evaluate");
    assert_eq!(after_microtasks, "error:true:#one|microtask");
}

#[test]
fn reset_navigation_history_preserves_current_entry_and_disposes_pruned_entries() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    vm.eval(
        r##"
(() => {
  history.pushState({ step: 1 }, "", "#one");
  history.pushState({ step: 2 }, "", "#two");
  globalThis.__lmResetHistoryEntries = navigation.entries();
  globalThis.__lmResetHistoryCurrent = navigation.currentEntry;
  globalThis.__lmResetHistoryDisposed = [];
  __lmResetHistoryEntries.forEach((entry, index) => {
    entry.addEventListener("dispose", () => __lmResetHistoryDisposed.push(index));
  });
})()
"##,
    )
    .expect("reset history setup should evaluate");

    assert!(
        vm.reset_navigation_history()
            .expect("reset history command should execute")
    );
    assert_eq!(
        vm.eval(
            r##"
JSON.stringify({
  historyLength: history.length,
  navigationLength: navigation.entries().length,
  sameCurrent: navigation.currentEntry === __lmResetHistoryCurrent,
  sameArrayEntry: navigation.entries()[0] === __lmResetHistoryCurrent,
  currentIndex: navigation.currentEntry.index,
  currentUrl: navigation.currentEntry.url,
  historyState: history.state.step,
  disposed: __lmResetHistoryDisposed
})
"##,
        )
        .expect("reset history result should evaluate"),
        r##"{"historyLength":1,"navigationLength":1,"sameCurrent":true,"sameArrayEntry":true,"currentIndex":0,"currentUrl":"https://example.com/base#two","historyState":2,"disposed":[1,0]}"##
    );
}

#[tokio::test]
async fn reset_navigation_history_updates_all_live_window_realms() {
    let mut vm = new_storage_test_vm("https://reset-history-realms.test/page.html");

    vm.eval(
        r##"
(() => {
  history.pushState({ realm: "top-default" }, "", "#top-default");
  const frame = document.createElement("iframe");
  frame.srcdoc = "<!doctype html><p>child</p>";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"##,
    )
    .expect("reset history frame setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .expect("reset history child realm should be materialized")
        .context_id;
    let child_frame_id = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("reset history child realm should exist")
        .frame_id
        .clone();
    vm.eval_in_child_default_context(
        child_context_id,
        r##"history.pushState({ realm: "child-default" }, "", "#child-default")"##,
    )
    .expect("child default history setup should evaluate");

    let top_isolated_context_id = vm
        .create_isolated_world("reset-history-top-isolated", false)
        .expect("top isolated world should be created");
    let child_isolated_context_id = vm
        .create_isolated_world_for_frame(&child_frame_id, "reset-history-child-isolated", false)
        .expect("child isolated world should be created");
    vm.eval_in_isolated_context(
        top_isolated_context_id,
        r##"history.pushState({ realm: "top-isolated" }, "", "#top-isolated")"##,
    )
    .expect("top isolated history setup should evaluate");
    vm.eval_in_isolated_context(
        child_isolated_context_id,
        r##"history.pushState({ realm: "child-isolated" }, "", "#child-isolated")"##,
    )
    .expect("child isolated history setup should evaluate");

    const INSTALL_ENTRY_OBSERVERS: &str = r#"
(() => {
  globalThis.__lmResetRealmEntries = navigation.entries();
  globalThis.__lmResetRealmCurrent = navigation.currentEntry;
  globalThis.__lmResetRealmDisposed = [];
  __lmResetRealmEntries.forEach((entry, index) => {
    entry.addEventListener("dispose", () => __lmResetRealmDisposed.push(index));
  });
})()
"#;
    vm.eval(INSTALL_ENTRY_OBSERVERS)
        .expect("top default reset observers should install");
    vm.eval_in_child_default_context(child_context_id, INSTALL_ENTRY_OBSERVERS)
        .expect("child default reset observers should install");
    vm.eval_in_isolated_context(top_isolated_context_id, INSTALL_ENTRY_OBSERVERS)
        .expect("top isolated reset observers should install");
    vm.eval_in_isolated_context(child_isolated_context_id, INSTALL_ENTRY_OBSERVERS)
        .expect("child isolated reset observers should install");

    vm.eval(
        r#"
(() => {
  const child = document.querySelector("iframe").contentWindow;
  globalThis.__lmResetRealmOrder = [];
  globalThis.__lmResetTopLengthBeforeDispose = history.length;
  globalThis.__lmResetChildLengthBeforeDispose = child.history.length;
  __lmResetRealmEntries[0].addEventListener("dispose", () => {
    __lmResetRealmOrder.push({
      listener: "top",
      lengthsUnchanged:
        history.length === __lmResetTopLengthBeforeDispose &&
        child.history.length === __lmResetChildLengthBeforeDispose,
      topEntries: navigation.entries().length,
      childEntries: child.navigation.entries().length
    });
  });
})()
"#,
    )
    .expect("top default reset order observer should install");
    vm.eval_in_child_default_context(
        child_context_id,
        r#"
__lmResetRealmEntries[0].addEventListener("dispose", () => {
  parent.__lmResetRealmOrder.push({
    listener: "child",
    lengthsUnchanged:
      history.length === parent.__lmResetChildLengthBeforeDispose &&
      parent.history.length === parent.__lmResetTopLengthBeforeDispose,
    topEntries: parent.navigation.entries().length,
    childEntries: navigation.entries().length
  });
});
"#,
    )
    .expect("child default reset order observer should install");

    assert!(
        vm.reset_navigation_history()
            .expect("multi-realm reset history command should execute")
    );

    const RESET_REALM_STATE: &str = r#"
JSON.stringify({
  historyLength: history.length,
  navigationLength: navigation.entries().length,
  sameCurrent: navigation.currentEntry === __lmResetRealmCurrent,
  sameArrayEntry: navigation.entries()[0] === __lmResetRealmCurrent,
  currentIndex: navigation.currentEntry.index,
  stateRealm: history.state.realm,
  disposedAll:
    __lmResetRealmDisposed.length === __lmResetRealmEntries.length - 1,
  disposedInReverseOrder:
    __lmResetRealmDisposed.every(
      (entryIndex, index, disposed) =>
        index === 0 || disposed[index - 1] > entryIndex
    )
})
"#;
    assert_eq!(
        vm.eval(RESET_REALM_STATE)
            .expect("top default reset state should evaluate"),
        r#"{"historyLength":1,"navigationLength":1,"sameCurrent":true,"sameArrayEntry":true,"currentIndex":0,"stateRealm":"top-default","disposedAll":true,"disposedInReverseOrder":true}"#
    );
    assert_eq!(
        vm.eval_in_child_default_context(child_context_id, RESET_REALM_STATE)
            .expect("child default reset state should evaluate"),
        r#"{"historyLength":1,"navigationLength":1,"sameCurrent":true,"sameArrayEntry":true,"currentIndex":0,"stateRealm":"child-default","disposedAll":true,"disposedInReverseOrder":true}"#
    );
    assert_eq!(
        vm.eval_in_isolated_context(top_isolated_context_id, RESET_REALM_STATE)
            .expect("top isolated reset state should evaluate"),
        r#"{"historyLength":1,"navigationLength":1,"sameCurrent":true,"sameArrayEntry":true,"currentIndex":0,"stateRealm":"top-isolated","disposedAll":true,"disposedInReverseOrder":true}"#
    );
    assert_eq!(
        vm.eval_in_isolated_context(child_isolated_context_id, RESET_REALM_STATE)
            .expect("child isolated reset state should evaluate"),
        r#"{"historyLength":1,"navigationLength":1,"sameCurrent":true,"sameArrayEntry":true,"currentIndex":0,"stateRealm":"child-isolated","disposedAll":true,"disposedInReverseOrder":true}"#
    );
    assert_eq!(
        vm.eval("JSON.stringify(__lmResetRealmOrder)")
            .expect("cross-realm reset order should evaluate"),
        r#"[{"listener":"top","lengthsUnchanged":true,"topEntries":1,"childEntries":2},{"listener":"child","lengthsUnchanged":true,"topEntries":1,"childEntries":1}]"#
    );
}

#[tokio::test]
async fn reset_navigation_history_preserves_child_entry_created_by_top_dispose_listener() {
    let mut vm = new_storage_test_vm("https://reset-history-reentrant-frame.test/page.html");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.srcdoc = "<!doctype html><p>child</p>";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("reentrant frame reset setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .expect("reentrant reset child realm should be materialized")
        .context_id;

    vm.eval(r##"history.pushState({ realm: "top" }, "", "#top")"##)
        .expect("top reentrant reset history setup should evaluate");
    vm.eval_in_child_default_context(
        child_context_id,
        r##"
(() => {
  history.pushState({ realm: "child-before-reset" }, "", "#child-before-reset");
  globalThis.__lmChildCurrentBeforeReset = navigation.currentEntry;
  globalThis.__lmChildEntriesBeforeReset = navigation.entries();
  globalThis.__lmChildDisposed = [];
  __lmChildEntriesBeforeReset.forEach((entry, index) => {
    entry.addEventListener("dispose", () => __lmChildDisposed.push(index));
  });
})()
"##,
    )
    .expect("child reentrant reset history setup should evaluate");
    vm.eval(
        r##"
navigation.entries()[0].addEventListener("dispose", () => {
  document.querySelector("iframe").contentWindow.history.pushState(
    { realm: "child-during-top-dispose" },
    "",
    "#child-during-top-dispose"
  );
});
"##,
    )
    .expect("top dispose reentrant child navigation should install");

    assert!(
        vm.reset_navigation_history()
            .expect("reentrant frame reset history command should execute")
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            child_context_id,
            r#"
JSON.stringify({
  historyLength: history.length,
  navigationLength: navigation.entries().length,
  currentIndex: navigation.currentEntry.index,
  retainedPreviousCurrent:
    navigation.entries()[0] === __lmChildCurrentBeforeReset,
  appendedCurrent:
    navigation.entries()[1] === navigation.currentEntry,
  stateRealm: history.state.realm,
  disposed: __lmChildDisposed
})
"#,
        )
        .expect("reentrant child reset history state should evaluate"),
        r#"{"historyLength":1,"navigationLength":2,"currentIndex":1,"retainedPreviousCurrent":true,"appendedCurrent":true,"stateRealm":"child-during-top-dispose","disposed":[0]}"#
    );
}

#[tokio::test]
async fn reset_navigation_history_updates_prebootstrapped_child_default_realm() {
    let mut vm = new_storage_test_vm("https://reset-history-prebootstrap.test/page.html");

    vm.eval(
        r##"
(() => {
  history.pushState({ step: 1 }, "", "#one");
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  void frame.contentWindow;
})()
"##,
    )
    .expect("prebootstrapped child reset setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "prebootstrapped child reset setup",
    )
    .await;
    assert_eq!(
        vm.prebootstrapped_child_default_contexts.borrow().len(),
        1,
        "contentWindow access should prebootstrap one child default context"
    );
    assert_eq!(
        vm.child_frame_realm_store.len(),
        0,
        "the child default context should not be materialized before reset"
    );

    assert!(
        vm.reset_navigation_history()
            .expect("prebootstrapped child reset history command should execute")
    );
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "prebootstrapped child after reset",
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            child_context_id,
            "JSON.stringify({ historyLength: history.length, navigationLength: navigation.entries().length, currentIndex: navigation.currentEntry.index })",
        )
        .expect("materialized child reset history state should evaluate"),
        r#"{"historyLength":1,"navigationLength":1,"currentIndex":0}"#
    );
}

#[tokio::test]
async fn history_methods_from_child_realm_traverse_receiver_history() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://history-cross-realm.test/page.html",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.setAttribute("sandbox", "allow-scripts allow-same-origin");
  frame.srcdoc = "<p>child</p>";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("cross-realm History frame setup should evaluate");
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("child setup should use the selected-task dispatcher");

    let setup = vm
        .eval(
            r##"
(() => {
  globalThis.__lmCrossRealmHistoryEvents = [];
  onpopstate = () => __lmCrossRealmHistoryEvents.push(location.hash || "initial");
  history.pushState({ step: 1 }, "", "#one");
  history.pushState({ step: 2 }, "", "#two");
  document.querySelector("iframe").contentWindow.history.back.call(history);
  return location.hash;
})()
"##,
        )
        .expect("borrowed child History.back should queue against the receiver");
    assert_eq!(setup, "#two");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "`${location.hash}|${__lmCrossRealmHistoryEvents.join(',')}`",
        "#one|#one",
        "borrowed child History.back should traverse the receiver history",
    )
    .await;
    assert_eq!(
        vm.eval("`${location.hash}|${__lmCrossRealmHistoryEvents.join(',')}`")
            .expect("borrowed child History.back result should evaluate"),
        "#one|#one"
    );

    vm.eval(
        r#"
document.querySelector("iframe").contentWindow.history.forward.call(history);
"#,
    )
    .expect("borrowed child History.forward should queue against the receiver");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "`${location.hash}|${__lmCrossRealmHistoryEvents.join(',')}`",
        "#two|#one,#two",
        "borrowed child History.forward should traverse the receiver history",
    )
    .await;
    assert_eq!(
        vm.eval("`${location.hash}|${__lmCrossRealmHistoryEvents.join(',')}`")
            .expect("borrowed child History.forward result should evaluate"),
        "#two|#one,#two"
    );

    vm.eval(
        r#"
document.querySelector("iframe").contentWindow.history.go.call(history, -2);
"#,
    )
    .expect("borrowed child History.go should queue against the receiver");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "`${location.hash}|${__lmCrossRealmHistoryEvents.join(',')}`",
        "|#one,#two,initial",
        "borrowed child History.go should traverse the receiver history",
    )
    .await;
    assert_eq!(
        vm.eval("`${location.hash}|${__lmCrossRealmHistoryEvents.join(',')}`")
            .expect("borrowed child History.go result should evaluate"),
        "|#one,#two,initial"
    );
}

#[tokio::test]
async fn traversal_navigate_destination_index_tracks_entry_identity() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://example.com/base", &loader);

    let before_back = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmTraversalDestinationProbe = { log: [] };
              const startIndex = navigation.currentEntry.index;
              navigation.navigate("#1");
              navigation.addEventListener("navigate", e => {
                globalThis.__lmTraversalDestinationProbe.backDestination = e.destination;
                globalThis.__lmTraversalDestinationProbe.log.push(`back:${startIndex}:${e.destination.index}`);
              }, { once: true });
              navigation.back().finished.then(
                () => globalThis.__lmTraversalDestinationProbe.log.push("backFinished"),
                error => globalThis.__lmTraversalDestinationProbe.log.push(`backRejected:${error.name}`)
              );
              return globalThis.__lmTraversalDestinationProbe.log.join("|");
            })()
            "##,
        )
        .expect("traversal destination setup should evaluate");
    assert_eq!(before_back, "");

    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .expect("queued history traversal should run")
    );
    let back_finished = vm
        .eval("globalThis.__lmTraversalDestinationProbe.log.join('|')")
        .expect("back traversal promise should be inspectable");
    assert_eq!(back_finished, "back:0:0|backFinished");

    let after_replace = vm
        .eval(
            r##"
            (() => {
              navigation.navigate("#clobber_back", { history: "replace" });
              return `${location.hash}:${globalThis.__lmTraversalDestinationProbe.backDestination.index}`;
            })()
            "##,
        )
        .expect("replace after traversal should evaluate");
    assert_eq!(after_replace, "#clobber_back:-1");

    let forward = vm
        .eval(
            r##"
            (() => {
              const probe = globalThis.__lmTraversalDestinationProbe;
              navigation.addEventListener("navigate", e => {
                probe.forwardInitial = e.destination.index;
                navigation.navigate("#clobber_forward");
                probe.forwardAfterNestedNavigate = e.destination.index;
              }, { once: true });
              const result = navigation.forward();
              result.committed.catch(error => { probe.forwardCommittedRejected = error.name; });
              result.finished.catch(error => { probe.forwardFinishedRejected = error.name; });
              return JSON.stringify({
                initial: probe.forwardInitial,
                after: probe.forwardAfterNestedNavigate,
                committedRejected: probe.forwardCommittedRejected || null,
                finishedRejected: probe.forwardFinishedRejected || null
              });
            })()
            "##,
        )
        .expect("forward traversal destination should evaluate");
    assert_eq!(
        forward,
        r#"{"committedRejected":null,"finishedRejected":null}"#
    );
    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .expect("queued forward traversal should run")
    );
    let forward_rejections = vm
        .eval(
            r##"
            JSON.stringify({
              initial: globalThis.__lmTraversalDestinationProbe.forwardInitial,
              after: globalThis.__lmTraversalDestinationProbe.forwardAfterNestedNavigate,
              committedRejected: globalThis.__lmTraversalDestinationProbe.forwardCommittedRejected || null,
              finishedRejected: globalThis.__lmTraversalDestinationProbe.forwardFinishedRejected || null
            })
            "##,
        )
        .expect("forward traversal rejection microtasks should settle");
    assert_eq!(
        forward_rejections,
        r#"{"initial":1,"after":-1,"committedRejected":"AbortError","finishedRejected":"AbortError"}"#
    );

    let helper_style_wait = vm
        .eval(
            r##"
            (() => {
              const probe = globalThis.__lmTraversalDestinationProbe;
              const { promise: all, resolve, reject } = Promise.withResolvers();
              let remaining = 0;
              const result = navigation.forward();
              for (const promise of [
                result.committed.then(
                  () => { probe.secondCommitted = "fulfilled"; },
                  error => { probe.secondCommitted = error.name; }
                ),
                result.finished.then(
                  () => { probe.secondFinished = "fulfilled"; },
                  error => { probe.secondFinished = error.name; }
                )
              ]) {
                remaining++;
                promise.then(() => {
                  --remaining;
                  if (!remaining) resolve("done");
                }, error => reject(error));
              }
              all.then(value => { probe.secondAll = value; }, error => { probe.secondAll = error.name; });
              return JSON.stringify({
                committed: probe.secondCommitted || null,
                finished: probe.secondFinished || null,
                all: probe.secondAll || null
              });
            })()
            "##,
        )
        .expect("helper-style wait probe should evaluate");
    assert_eq!(
        helper_style_wait,
        r#"{"committed":null,"finished":null,"all":null}"#
    );
    let helper_style_wait_after_microtasks = vm
        .eval(
            r##"
            JSON.stringify({
              committed: globalThis.__lmTraversalDestinationProbe.secondCommitted || null,
              finished: globalThis.__lmTraversalDestinationProbe.secondFinished || null,
              all: globalThis.__lmTraversalDestinationProbe.secondAll || null
            })
            "##,
        )
        .expect("helper-style wait microtasks should settle");
    assert_eq!(
        helper_style_wait_after_microtasks,
        r#"{"committed":"InvalidStateError","finished":"InvalidStateError","all":"done"}"#
    );
}
#[tokio::test]
async fn repeated_traverse_to_reuses_pending_navigation_promises() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              const key = navigation.currentEntry.key;
              navigation.navigate("#one");
              const first = navigation.traverseTo(key);
              const second = navigation.traverseTo(key);
              globalThis.__lmRepeatedTraverseTo = { first, second, log: [] };
              first.finished.then(
                entry => globalThis.__lmRepeatedTraverseTo.log.push(`finished:${entry.url}:${location.hash}`),
                error => globalThis.__lmRepeatedTraverseTo.log.push(`rejected:${error.name}`)
              );
              return [
                first !== second,
                first.committed === second.committed,
                first.finished === second.finished
              ].join("|");
            })()
            "##,
        )
        .expect("repeated traverseTo setup should evaluate");

    assert_eq!(setup, "true|true|true");
    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .expect("queued repeated traverseTo should run")
    );
    let settled = vm
        .eval("globalThis.__lmRepeatedTraverseTo.log.join('|')")
        .expect("repeated traverseTo settlement should evaluate");
    assert_eq!(settled, "finished:https://example.com/base:");
}
