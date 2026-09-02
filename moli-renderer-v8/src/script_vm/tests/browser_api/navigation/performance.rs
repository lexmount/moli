use super::*;

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
                  Object.getOwnPropertyDescriptor(PerformanceNavigationTiming.prototype, name)
                  ?? Object.getOwnPropertyDescriptor(PerformanceResourceTiming.prototype, name);
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
                isResourceTiming: navigation instanceof PerformanceResourceTiming,
                constructorParent: Object.getPrototypeOf(PerformanceNavigationTiming) === PerformanceResourceTiming,
                prototypeParent: Object.getPrototypeOf(PerformanceNavigationTiming.prototype) === PerformanceResourceTiming.prototype,
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
        r#"{"ctor":"function","name":"https://performance-navigation-timing.test/","entryType":"navigation","isNavigationTiming":true,"isResourceTiming":true,"constructorParent":true,"prototypeParent":true,"inheritsPerformanceEntry":true,"prototypeAttributeNames":"unloadEventStart,unloadEventEnd,domInteractive,domContentLoadedEventStart,domContentLoadedEventEnd,domComplete,loadEventStart,loadEventEnd,type,redirectCount","descriptorsStable":true,"constructError":"TypeError"}"#
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
              const getterResult = (getter, receiver) => {
                try {
                  return stringify(getter.call(receiver));
                } catch (error) {
                  return error.name;
                }
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
                fakeName: getterResult(entryNameGetter, fake),
                fakeNavigationType: getterResult(navTypeGetter, fake),
                fakeLoadEventEnd: getterResult(navLoadGetter, fake),
                fakeResourceValues: resourceGetters.map(getter => getterResult(getter, {})).join("|")
              });
            })()
            "#,
        )
        .expect("performance entry slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"initialNavigationNames":[],"initialMarkNames":[],"initialMeasureNames":[],"markName":"real-mark","markEntryType":"mark","markStartSpoofIgnored":true,"markDuration":0,"markDetailNull":true,"measureDetail":"real","entryDescriptors":["name:true:function:get name:0:undefined:true:true:false","entryType:true:function:get entryType:0:undefined:true:true:false","startTime:true:function:get startTime:0:undefined:true:true:false","duration:true:function:get duration:0:undefined:true:true:false"],"detailDescriptors":["detail:true:function:get detail:0:undefined:true:true:false","detail:true:function:get detail:0:undefined:true:true:false"],"resourceDescriptors":["initiatorType:true:function:get initiatorType:0:undefined:true:true:false","nextHopProtocol:true:function:get nextHopProtocol:0:undefined:true:true:false","workerStart:true:function:get workerStart:0:undefined:true:true:false","redirectStart:true:function:get redirectStart:0:undefined:true:true:false","redirectEnd:true:function:get redirectEnd:0:undefined:true:true:false","fetchStart:true:function:get fetchStart:0:undefined:true:true:false","domainLookupStart:true:function:get domainLookupStart:0:undefined:true:true:false","domainLookupEnd:true:function:get domainLookupEnd:0:undefined:true:true:false","connectStart:true:function:get connectStart:0:undefined:true:true:false","connectEnd:true:function:get connectEnd:0:undefined:true:true:false","secureConnectionStart:true:function:get secureConnectionStart:0:undefined:true:true:false","requestStart:true:function:get requestStart:0:undefined:true:true:false","responseStart:true:function:get responseStart:0:undefined:true:true:false","responseEnd:true:function:get responseEnd:0:undefined:true:true:false","transferSize:true:function:get transferSize:0:undefined:true:true:false","encodedBodySize:true:function:get encodedBodySize:0:undefined:true:true:false","decodedBodySize:true:function:get decodedBodySize:0:undefined:true:true:false","renderBlockingStatus:true:function:get renderBlockingStatus:0:undefined:true:true:false","responseStatus:true:function:get responseStatus:0:undefined:true:true:false","contentType:true:function:get contentType:0:undefined:true:true:false"],"navigationType":"navigate","navigationLoadEventEnd":0,"navigationDuration":0,"byRealMark":1,"bySpoofMark":0,"fakeName":"TypeError","fakeNavigationType":"TypeError","fakeLoadEventEnd":"TypeError","fakeResourceValues":"TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError"}"#
    );
}

#[test]
fn performance_entry_ids_follow_the_current_navigation() {
    let mut vm = new_storage_test_vm("https://performance-entry-identity.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const navigation = performance.getEntriesByType("navigation")[0];
              const mark = performance.mark("identity-mark", { startTime: 1 });
              const measure = performance.measure("identity-measure", {
                start: 1,
                duration: 2
              });
              const detached = new PerformanceMark("detached", { startTime: 3 });
              const markJson = mark.toJSON();
              const descriptors = ["id", "navigationId"].map(name => {
                const descriptor = Object.getOwnPropertyDescriptor(
                  PerformanceEntry.prototype,
                  name
                );
                return [
                  name,
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.get?.length,
                  typeof descriptor?.set,
                  descriptor?.enumerable,
                  descriptor?.configurable
                ].join(":");
              });
              return JSON.stringify({
                descriptors,
                navigation:
                  Number.isSafeInteger(navigation.id)
                  && navigation.id > 0
                  && navigation.navigationId === navigation.id,
                queued:
                  Number.isSafeInteger(mark.id)
                  && Number.isSafeInteger(measure.id)
                  && navigation.id < mark.id
                  && mark.id < measure.id
                  && mark.navigationId === navigation.id
                  && measure.navigationId === navigation.id,
                detached: detached.id === 0 && detached.navigationId === 0,
                json:
                  Object.keys(markJson).join(",") ===
                    "id,name,entryType,startTime,duration,navigationId"
                  && markJson.id === mark.id
                  && markJson.navigationId === navigation.id
              });
            })()
            "#,
        )
        .expect("PerformanceEntry identity probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":["id:function:get id:0:undefined:true:true","navigationId:function:get navigationId:0:undefined:true:true"],"navigation":true,"queued":true,"detached":true,"json":true}"#
    );
}

#[test]
fn performance_user_timing_enforces_mark_and_measure_boundaries() {
    let mut vm = new_storage_test_vm("https://performance-user-timing-boundaries.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const capture = callback => {
                try {
                  callback();
                  return "none";
                } catch (error) {
                  return `${error.name}:${error.code}`;
                }
              };
              performance.mark("later", { startTime: 5 });
              const sourceDetail = { value: 1 };
              const cloned = performance.measure("cloned", {
                start: 1,
                duration: 2,
                detail: sourceDetail
              });
              sourceDetail.value = 9;
              const negative = performance.measure(
                "negative-duration",
                "later",
                "navigationStart"
              );
              const legacyPendingNames = [
                "unloadEventStart",
                "unloadEventEnd",
                "redirectStart",
                "redirectEnd",
                "secureConnectionStart",
                "domInteractive",
                "domContentLoadedEventStart",
                "domContentLoadedEventEnd",
                "domComplete",
                "loadEventStart",
                "loadEventEnd"
              ];
              return JSON.stringify({
                reservedMark: capture(() => performance.mark("navigationStart")),
                missingMark: capture(() => performance.measure("missing", "does-not-exist")),
                numericLegacyMark: capture(() => performance.measure("number", 51.15, "later")),
                unavailableTiming: capture(() => performance.measure("pending", "redirectStart")),
                detailOnlyOptions: capture(() => performance.measure("detail-only", { detail: 1 })),
                overSpecifiedOptions: capture(() => performance.measure("all", {
                  start: 1,
                  duration: 2,
                  end: 3
                })),
                negativeMark: capture(() => performance.mark("bad-mark", { startTime: -1 })),
                infiniteMark: capture(() => performance.mark("bad-infinite", { startTime: Infinity })),
                negativeBoundary: capture(() => performance.measure("bad-boundary", { start: -1 })),
                negativeDuration: negative.duration,
                navigationStartTime: performance.measure("from-navigation", "navigationStart").startTime,
                pendingTimingStartsAtZero: legacyPendingNames.every(name => performance.timing[name] === 0),
                clonedDetail: `${cloned.detail.value}:${cloned.detail === sourceDetail}`,
                navigationEntryIsNotMark: capture(() => performance.measure("entry-name", location.href))
              });
            })()
            "#,
        )
        .expect("User Timing boundary probe should evaluate");

    assert_eq!(
        result,
        r#"{"reservedMark":"SyntaxError:12","missingMark":"SyntaxError:12","numericLegacyMark":"SyntaxError:12","unavailableTiming":"InvalidAccessError:15","detailOnlyOptions":"TypeError:undefined","overSpecifiedOptions":"TypeError:undefined","negativeMark":"TypeError:undefined","infiniteMark":"TypeError:undefined","negativeBoundary":"TypeError:undefined","negativeDuration":-5,"navigationStartTime":0,"pendingTimingStartsAtZero":true,"clonedDetail":"1:false","navigationEntryIsNotMark":"SyntaxError:12"}"#
    );
}

#[test]
fn performance_mark_constructor_creates_detached_structured_entries() {
    let mut vm = new_storage_test_vm("https://performance-mark-constructor.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const capture = callback => {
                try {
                  callback();
                  return "none";
                } catch (error) {
                  return `${error.name}:${error.code}`;
                }
              };
              const sourceDetail = { state: "before" };
              const entry = new PerformanceMark("detached", {
                startTime: 2,
                detail: sourceDetail
              });
              sourceDetail.state = "after";
              class DerivedPerformanceMark extends PerformanceMark {}
              const derived = new DerivedPerformanceMark("derived", { startTime: 3 });
              return JSON.stringify({
                shape: [
                  entry instanceof PerformanceEntry,
                  entry instanceof PerformanceMark,
                  Object.prototype.toString.call(entry),
                  entry.name,
                  entry.entryType,
                  entry.startTime,
                  entry.duration
                ].join(":"),
                detail: `${entry.detail.state}:${entry.detail === entry.detail}:${entry.detail === sourceDetail}`,
                timelineEntries: performance.getEntriesByName("detached", "mark").length,
                derived: [
                  derived instanceof DerivedPerformanceMark,
                  derived instanceof PerformanceMark,
                  Object.getPrototypeOf(derived) === DerivedPerformanceMark.prototype,
                  derived.name,
                  derived.startTime
                ].join(":"),
                constructorInheritance:
                  Object.getPrototypeOf(PerformanceMark) === PerformanceEntry
                  && Object.getPrototypeOf(PerformanceMeasure) === PerformanceEntry,
                detailBrand: capture(() =>
                  Object.getOwnPropertyDescriptor(PerformanceMark.prototype, "detail")
                    .get.call(PerformanceMark.prototype)),
                methodBrands: [
                  performance.mark,
                  performance.clearMarks,
                  performance.measure,
                  performance.clearMeasures
                ].map(method => capture(() => method.call(null, "unbound"))).join("|"),
                withoutNew: capture(() => PerformanceMark("call")),
                missingName: capture(() => new PerformanceMark()),
                negativeStart: capture(() => new PerformanceMark("negative", { startTime: -1 })),
                infiniteStart: capture(() => new PerformanceMark("infinite", { startTime: Infinity })),
                reservedName: capture(() => new PerformanceMark("navigationStart")),
                cloneError: capture(() => new PerformanceMark("clone", {
                  detail: { value: Symbol() }
                }))
              });
            })()
            "#,
        )
        .expect("PerformanceMark constructor probe should evaluate");

    assert_eq!(
        result,
        r#"{"shape":"true:true:[object PerformanceMark]:detached:mark:2:0","detail":"before:true:false","timelineEntries":0,"derived":"true:true:true:derived:3","constructorInheritance":true,"detailBrand":"TypeError:undefined","methodBrands":"TypeError:undefined|TypeError:undefined|TypeError:undefined|TypeError:undefined","withoutNew":"TypeError:undefined","missingName":"TypeError:undefined","negativeStart":"TypeError:undefined","infiniteStart":"TypeError:undefined","reservedName":"SyntaxError:12","cloneError":"DataCloneError:25"}"#
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
              const markJson = mark.toJSON();
              const measureJson = measure.toJSON();
              const identities =
                markJson.id === mark.id
                && markJson.navigationId === mark.navigationId
                && measureJson.id === measure.id
                && measureJson.navigationId === measure.navigationId;
              delete markJson.id;
              delete markJson.navigationId;
              delete measureJson.id;
              delete measureJson.navigationId;
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
                mark: markJson,
                measure: measureJson,
                identities,
                fakeError
              });
            })()
            "#,
        )
        .expect("PerformanceEntry toJSON probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptor":"toJSON:0:true:true:true","inherited":true,"mark":{"name":"json-mark","entryType":"mark","startTime":12,"duration":0},"measure":{"name":"json-measure","entryType":"measure","startTime":3,"duration":4},"identities":true,"fakeError":"TypeError"}"#
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
                b"Application/JavaScript; charset=utf-8".to_vec(),
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
                "id",
                "name",
                "entryType",
                "startTime",
                "duration",
                "navigationId",
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
                identity:
                  Number.isSafeInteger(entry.id)
                  && entry.id > 0
                  && entry.navigationId ===
                    performance.getEntriesByType("navigation")[0].id,
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
        r#"{"length":1,"name":"https://resource-timing-script.test/app.js","entryType":"resource","initiatorType":"script","instance":true,"responseStatus":200,"contentType":"application/javascript","renderBlockingStatusValid":true,"transferSizePositive":true,"encodedBodySize":7,"decodedBodySize":7,"identity":true,"jsonOwnKeys":true,"jsonMatchesEntry":true,"toJSONDescriptor":"toJSON:0:true:true:true","fakeToJSONError":"TypeError"}"#
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
              const capture = callback => {
                try {
                  return String(callback());
                } catch (error) {
                  return error.name;
                }
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
                fakeTimeOrigin: capture(() => timeOriginGetter.call(fakePerformance)),
                fakeTiming: capture(() => timingGetter.call(fakePerformance)),
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
        r#"{"initialPerformanceNames":[],"initialNavigationNames":[],"initialEventCountsNames":[],"timeOriginSpoofIgnored":true,"timingStable":true,"navigationStable":true,"eventCountsStable":true,"performanceDescriptorsStable":true,"entriesSpoofIgnored":1,"navigationType":0,"navigationRedirectCount":0,"navigationDescriptorsStable":true,"eventCountsClick":0,"eventCountsFirstValue":0,"eventCountsFirstEntry":"auxclick:0","jsonTimeOriginStable":true,"jsonNavigationType":0,"fakeTimeOrigin":"TypeError","fakeTiming":"TypeError","fakeNavigationType":"undefined","fakeEventCountsGet":"0","fakeEventCountsValue":"0"}"#
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
                  methodSummary(PerformanceNavigationTiming.prototype, "toJSON")
                ]
              });
            })()
            "#,
        )
        .expect("performance toJSON snapshot probe should evaluate");

    assert_eq!(
        result,
        r#"{"timeOriginNumber":true,"timeOriginEnumerable":true,"timingEnumerable":true,"navigationEnumerable":true,"timingSnapshot":true,"navigationSnapshot":true,"navigationEntrySnapshot":true,"timingOwn":true,"navigationOwn":true,"toJsonDescriptors":["toJSON:function:toJSON:0:false:true:true:true","toJSON:function:toJSON:0:false:true:true:true","toJSON:function:toJSON:0:true:true:true:true"]}"#
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
fn performance_navigation_lifecycle_timestamps_are_write_once_per_navigation() {
    let mut vm = new_storage_test_vm("https://performance-navigation-write-once.test/");

    vm.dispatch_document_lifecycle_event("DOMContentLoaded")
        .expect("first DOMContentLoaded should update navigation timing");
    vm.dispatch_window_load_event()
        .expect("first load should update navigation timing");
    vm.eval(
        r#"
        globalThis.__initialNavigationLifecycleTiming = (() => {
          const navigation = performance.getEntriesByType("navigation")[0];
          return [
            navigation.domInteractive,
            navigation.domContentLoadedEventStart,
            navigation.domContentLoadedEventEnd,
            navigation.domComplete,
            navigation.loadEventStart,
            navigation.loadEventEnd
          ];
        })();
        "#,
    )
    .expect("initial navigation lifecycle timing should be captured");

    vm.dispatch_document_lifecycle_event("DOMContentLoaded")
        .expect("repeated DOMContentLoaded should remain dispatchable");
    vm.dispatch_window_load_event()
        .expect("repeated load should remain dispatchable");

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const navigation = performance.getEntriesByType("navigation")[0];
              const current = [
                navigation.domInteractive,
                navigation.domContentLoadedEventStart,
                navigation.domContentLoadedEventEnd,
                navigation.domComplete,
                navigation.loadEventStart,
                navigation.loadEventEnd
              ];
              return current.every(
                (value, index) =>
                  value === globalThis.__initialNavigationLifecycleTiming[index]
              );
            })()
            "#,
        )
        .expect("repeated navigation lifecycle timing should be compared"),
        "true"
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
