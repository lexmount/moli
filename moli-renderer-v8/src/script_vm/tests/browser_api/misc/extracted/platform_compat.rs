use super::*;

#[test]
fn zhihu_capability_probe_fixture_exposes_expected_probe_keyset() {
    let value = eval_probe_fixture_output(
        "https://probe.test/zhihu-capability-probe.html",
        ZHIHU_CAPABILITY_PROBE_HTML,
    );

    let keys = value
        .as_object()
        .expect("capability probe output should be an object")
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let expected = [
        "audio",
        "canvasContextCtor",
        "canvasCtor",
        "canvasGetContextType",
        "canvasTag",
        "canvasToDataURLType",
        "createElementChainLastTag",
        "createElementTagMismatchCount",
        "crypto",
        "customElementsType",
        "documentAllCtor",
        "documentAllLengthType",
        "documentAllLooseUndefined",
        "documentAllStrictUndefined",
        "documentAllToString",
        "documentAllType",
        "documentAllValueType",
        "errorStackGetterTriggered",
        "globalType",
        "historyLocation",
        "htmlAllCollectionCheck",
        "mediaDevices",
        "mediaSource",
        "mimeTypes",
        "navigatorInfo",
        "newDocumentAllCtor",
        "newDocumentAllLengthType",
        "newDocumentAllString",
        "newDocumentAllTag",
        "newDocumentAllType",
        "phantomFlags",
        "plugins",
        "processType",
        "screenInfo",
        "storage",
        "timers",
        "touch",
        "userAgent",
        "webdriver",
        "webgl",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(keys, expected);
}
#[test]
fn zhihu_capability_probe_fixture_matches_stable_moli_baseline() {
    let value = eval_probe_fixture_output(
        "https://probe.test/zhihu-capability-probe.html",
        ZHIHU_CAPABILITY_PROBE_HTML,
    );

    let navigator = value["navigatorInfo"]
        .as_object()
        .expect("navigatorInfo should be an object");
    assert_eq!(value["userAgent"], DEFAULT_USER_AGENT);
    assert_eq!(
        navigator.get("platform"),
        Some(&serde_json::Value::String(
            DEFAULT_WINDOW_SURFACE_PROFILE.platform.to_owned()
        ))
    );
    assert_eq!(
        navigator.get("language"),
        Some(&serde_json::Value::String("en-US".to_owned()))
    );
    assert_eq!(
        navigator.get("languages"),
        Some(&serde_json::json!(["en-US", "en"]))
    );
    assert_eq!(
        navigator.get("vendor"),
        Some(&serde_json::Value::String("Google Inc.".to_owned()))
    );
    assert_eq!(
        navigator.get("hardwareConcurrency"),
        Some(&serde_json::Value::from(4))
    );
    assert_eq!(
        navigator.get("maxTouchPoints"),
        Some(&serde_json::Value::from(0))
    );
    assert_eq!(
        navigator.get("deviceMemory"),
        Some(&serde_json::Value::from(8))
    );
    assert_eq!(
        navigator.get("pdfViewerEnabled"),
        Some(&serde_json::Value::Bool(true))
    );
    assert_eq!(navigator.get("doNotTrack"), Some(&serde_json::Value::Null));

    assert_eq!(value["plugins"]["ctor"], "PluginArray");
    assert_eq!(value["plugins"]["length"], 5);
    assert_eq!(value["mimeTypes"]["ctor"], "MimeTypeArray");
    assert_eq!(value["mimeTypes"]["length"], 2);
    assert_eq!(value["customElementsType"], "object");

    assert_eq!(value["documentAllType"], "undefined");
    assert_eq!(value["documentAllValueType"], "undefined");
    assert_eq!(value["documentAllLooseUndefined"], true);
    assert_eq!(value["documentAllStrictUndefined"], false);
    assert_eq!(value["documentAllLengthType"], "number");
    assert_eq!(value["documentAllToString"], "[object HTMLAllCollection]");
    assert_eq!(value["createElementTagMismatchCount"], 0);
    assert_eq!(value["createElementChainLastTag"], "A");

    assert_eq!(value["canvasCtor"], "HTMLCanvasElement");
    assert_eq!(value["canvasContextCtor"], "CanvasRenderingContext2D");
    assert_eq!(value["canvasToDataURLType"], "function");
    assert_eq!(value["webgl"]["ctor"], "WebGLRenderingContext");
    assert_eq!(value["webgl"]["getParameterType"], "function");
    assert_eq!(value["webdriver"], false);

    let phantom_flags = value["phantomFlags"]
        .as_object()
        .expect("phantomFlags should be an object");
    for key in [
        "__phantomas",
        "_phantom",
        "WebPage",
        "fxdriver_id",
        "__fxdriver_unwrapped",
        "ubot",
        "CasperError",
        "casper",
        "patchRequire",
        "cdc",
        "__webdriver_script_fn",
        "_resourceLoader",
        "_sessionHistory",
        "_virtualConsole",
    ] {
        assert_eq!(
            phantom_flags.get(key),
            Some(&serde_json::Value::String("undefined".to_owned())),
            "unexpected automation marker state for {key}"
        );
    }

    assert_eq!(value["storage"]["localStorage"], "object");
    assert_eq!(value["storage"]["sessionStorage"], "object");
    assert_eq!(value["storage"]["lsGetItem"], "function");
    assert_eq!(value["errorStackGetterTriggered"], 1);
    assert_eq!(value["processType"], "undefined");
    assert_eq!(value["globalType"], "undefined");

    assert_eq!(value["screenInfo"]["availWidth"], 1920);
    assert_eq!(value["screenInfo"]["availHeight"], 1080);
    assert_eq!(value["screenInfo"]["colorDepth"], 24);
    assert_eq!(value["screenInfo"]["pixelDepth"], 24);
    assert_eq!(value["screenInfo"]["devicePixelRatio"], 1);

    assert_eq!(value["timers"]["setTimeout"], "function");
    assert_eq!(value["timers"]["setInterval"], "function");
    assert_eq!(value["timers"]["stop"], "function");
    assert_eq!(value["timers"]["print"], "function");
    assert_eq!(value["timers"]["open"], "function");
    assert_eq!(value["timers"]["topEqSelf"], true);

    assert_eq!(value["mediaDevices"]["ctor"], "MediaDevices");
    assert_eq!(value["mediaDevices"]["enumerateDevicesType"], "function");
    assert_eq!(value["audio"]["contextCtor"], "OfflineAudioContext");
    assert_eq!(value["audio"]["startRenderingType"], "function");
    assert_eq!(value["touch"]["touchEvent"], "function");
    assert_eq!(value["touch"]["touchCtor"], "function");
    assert_eq!(value["mediaSource"]["MediaSourceType"], "function");
    assert_eq!(value["mediaSource"]["avc1"], true);
    assert_eq!(value["crypto"]["cryptoType"], "object");
    assert_eq!(value["crypto"]["getRandomValuesType"], "function");
    let sample = value["crypto"]["sample"]
        .as_array()
        .expect("crypto sample should be an array");
    assert_eq!(sample.len(), 4);
    assert!(
        sample
            .iter()
            .any(|entry: &serde_json::Value| entry.as_u64().unwrap_or_default() != 0),
        "crypto sample should not be all zeros"
    );

    assert_eq!(value["historyLocation"]["pushState"], "function");
    assert_eq!(value["historyLocation"]["replaceState"], "function");
    assert_eq!(
        value["historyLocation"]["href"],
        "https://probe.test/zhihu-capability-probe.html"
    );
    assert_eq!(value["historyLocation"]["hostname"], "probe.test");
}
#[test]
fn zhihu_bot_detection_harness_fixture_exposes_expected_sections() {
    let value = eval_probe_fixture_output(
        "https://probe.test/zhihu-bot-detection-harness.html",
        ZHIHU_BOT_DETECTION_HARNESS_HTML,
    );

    let keys = value
        .as_object()
        .expect("bot detection harness output should be an object")
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let expected = [
        "automationMarkers",
        "canvasWebgl",
        "descriptors",
        "documentAll",
        "globalFunctions",
        "mediaSource",
        "nativeSurface",
        "navigatorProfile",
        "pluginsMimeTypes",
        "screenTouch",
        "serverEscape",
        "storage",
        "windowAliases",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(keys, expected);

    let mut errors = Vec::new();
    collect_probe_error_paths(&value, "", &mut errors);
    assert!(
        errors.is_empty(),
        "bot detection harness should not surface probe errors: {errors:#?}"
    );
}
#[test]
fn zhihu_bot_detection_harness_fixture_matches_stable_moli_baseline() {
    let value = eval_probe_fixture_output(
        "https://probe.test/zhihu-bot-detection-harness.html",
        ZHIHU_BOT_DETECTION_HARNESS_HTML,
    );

    assert_eq!(value["windowAliases"]["windowEqGlobalThis"], true);
    assert_eq!(value["windowAliases"]["selfEqGlobalThis"], true);
    assert_eq!(value["windowAliases"]["topEqGlobalThis"], true);
    assert_eq!(value["windowAliases"]["parentEqGlobalThis"], true);
    assert_eq!(value["windowAliases"]["framesEqGlobalThis"], true);
    assert_eq!(value["windowAliases"]["namedWindowType"], "object");
    assert_eq!(value["windowAliases"]["namedProbeHit"], true);

    for key in [
        "__phantomas",
        "_phantom",
        "WebPage",
        "fxdriver_id",
        "__fxdriver_unwrapped",
        "ubot",
        "CasperError",
        "casper",
        "$cdc_asdjflasutopfhvcZLmcfl_",
        "__webdriver_script_fn",
        "patchRequire",
    ] {
        assert_eq!(value["automationMarkers"][key], "undefined");
    }

    for key in [
        "_resourceLoader",
        "_sessionHistory",
        "_virtualConsole",
        "_parent",
        "_origin",
        "_document",
        "process",
        "global",
    ] {
        assert_eq!(value["serverEscape"]["window"][key], "undefined");
    }
    for key in [
        "process",
        "global",
        "require",
        "module",
        "exports",
        "__dirname",
        "__filename",
        "executeUserEntryPoint",
        "node",
        "JSDOM",
        "ws",
    ] {
        assert_eq!(value["serverEscape"]["dynamic"][key], "undefined");
    }

    assert_eq!(value["globalFunctions"]["addEventListenerType"], "function");
    assert_eq!(
        value["globalFunctions"]["removeEventListenerType"],
        "function"
    );
    assert_eq!(value["globalFunctions"]["dispatchEventType"], "function");
    assert_eq!(value["globalFunctions"]["addEventListenerLength"], 2);
    assert_eq!(value["globalFunctions"]["removeEventListenerLength"], 2);
    assert_eq!(value["globalFunctions"]["dispatchEventLength"], 1);
    assert_eq!(value["globalFunctions"]["blurType"], "function");
    assert_eq!(value["globalFunctions"]["blurLength"], 0);
    assert_eq!(value["globalFunctions"]["findType"], "function");
    assert_eq!(value["globalFunctions"]["findLength"], 0);
    assert_eq!(value["globalFunctions"]["stopType"], "function");
    assert_eq!(value["globalFunctions"]["printType"], "function");
    assert_eq!(value["globalFunctions"]["openType"], "function");
    assert_eq!(value["globalFunctions"]["bareMatchesWindowAdd"], true);
    assert_eq!(value["globalFunctions"]["ownAddEventListener"], false);

    assert_eq!(value["descriptors"]["window"]["present"], true);
    assert_eq!(value["descriptors"]["window"]["configurable"], false);
    assert_eq!(value["descriptors"]["window"]["hasGetter"], true);
    assert_eq!(value["descriptors"]["document"]["present"], true);
    assert_eq!(value["descriptors"]["document"]["configurable"], false);
    assert_eq!(value["descriptors"]["document"]["hasGetter"], true);

    for key in [
        "open",
        "print",
        "stop",
        "blur",
        "find",
        "addEventListener",
        "removeEventListener",
        "dispatchEvent",
        "getRandomValues",
        "isTypeSupported",
        "documentAllGetter",
        "webdriverGetter",
        "screenAvailWidthGetter",
    ] {
        assert_eq!(
            value["nativeSurface"][key], true,
            "{key} should look native"
        );
    }

    assert_eq!(value["documentAll"]["type"], "undefined");
    assert_eq!(value["documentAll"]["looseUndefined"], true);
    assert_eq!(value["documentAll"]["strictUndefined"], false);
    assert_eq!(value["documentAll"]["lengthType"], "number");
    assert_eq!(value["documentAll"]["tag"], "[object HTMLAllCollection]");
    assert_eq!(value["documentAll"]["ctor"], serde_json::Value::Null);
    assert_eq!(value["documentAll"]["detachedType"], "object");
    assert_eq!(value["documentAll"]["detachedLengthType"], "undefined");
    assert_eq!(value["documentAll"]["detachedTag"], "[object Null]");
    assert_eq!(value["documentAll"]["detachedString"], "null");
    assert_eq!(value["documentAll"]["namedHit"], true);

    assert_eq!(value["navigatorProfile"]["userAgent"], DEFAULT_USER_AGENT);
    assert_eq!(value["navigatorProfile"]["vendor"], "Google Inc.");
    assert_eq!(
        value["navigatorProfile"]["platform"],
        DEFAULT_WINDOW_SURFACE_PROFILE.platform
    );
    assert_eq!(value["navigatorProfile"]["language"], "en-US");
    assert_eq!(
        value["navigatorProfile"]["languages"],
        serde_json::json!(["en-US", "en"])
    );
    assert_eq!(value["navigatorProfile"]["webdriver"], false);
    assert_eq!(value["navigatorProfile"]["hardwareConcurrency"], 4);
    assert_eq!(value["navigatorProfile"]["maxTouchPoints"], 0);
    assert_eq!(value["navigatorProfile"]["deviceMemory"], 8);
    assert_eq!(value["navigatorProfile"]["pdfViewerEnabled"], true);

    assert_eq!(value["pluginsMimeTypes"]["plugins"]["ctor"], "PluginArray");
    assert_eq!(value["pluginsMimeTypes"]["plugins"]["length"], 5);
    assert_eq!(
        value["pluginsMimeTypes"]["mimeTypes"]["ctor"],
        "MimeTypeArray"
    );
    assert_eq!(value["pluginsMimeTypes"]["mimeTypes"]["length"], 2);

    assert_eq!(value["screenTouch"]["width"], 1920);
    assert_eq!(value["screenTouch"]["height"], 1080);
    assert_eq!(value["screenTouch"]["availWidth"], 1920);
    assert_eq!(value["screenTouch"]["availHeight"], 1080);
    assert_eq!(value["screenTouch"]["availLeft"], 0);
    assert_eq!(value["screenTouch"]["availTop"], 0);
    assert_eq!(value["screenTouch"]["colorDepth"], 24);
    assert_eq!(value["screenTouch"]["pixelDepth"], 24);
    assert_eq!(value["screenTouch"]["orientationType"], "landscape-primary");
    assert_eq!(value["screenTouch"]["touchType"], "function");
    assert_eq!(value["screenTouch"]["touchEventType"], "function");
    assert_eq!(value["screenTouch"]["ontouchstartType"], "undefined");

    assert_eq!(value["canvasWebgl"]["canvasCtor"], "HTMLCanvasElement");
    assert_eq!(value["canvasWebgl"]["canvasTag"], "CANVAS");
    assert_eq!(value["canvasWebgl"]["getContextType"], "function");
    assert_eq!(value["canvasWebgl"]["toDataURLType"], "function");
    assert_eq!(
        value["canvasWebgl"]["context2dCtor"],
        "CanvasRenderingContext2D"
    );
    assert_eq!(value["canvasWebgl"]["webglCtor"], serde_json::Value::Null);
    assert_eq!(value["canvasWebgl"]["webglGetParameterType"], "undefined");

    assert_eq!(value["mediaSource"]["ctorType"], "function");
    assert_eq!(value["mediaSource"]["name"], "MediaSource");
    assert_eq!(value["mediaSource"]["staticName"], "isTypeSupported");
    assert_eq!(value["mediaSource"]["avc1"], true);
    assert_eq!(value["mediaSource"]["vp09"], false);
    assert_eq!(value["mediaSource"]["instanceOf"], true);
    assert_eq!(value["mediaSource"]["prototypeMatches"], true);
    assert_eq!(value["mediaSource"]["ownKeys"], serde_json::json!([]));

    assert_eq!(value["storage"]["localStorageType"], "object");
    assert_eq!(value["storage"]["sessionStorageType"], "object");
    assert_eq!(value["storage"]["localStorageGetItemType"], "function");
    assert_eq!(value["storage"]["sessionStorageGetItemType"], "function");
}
#[tokio::test]
async fn webassembly_compile_accepts_spec_valid_bounds_above_v8_instantiation_limits() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new(
        url::Url::parse("https://wasm-declared-limits.test/").expect("test URL"),
        &loader,
    );
    vm.document_runtime.note_dom_content_loaded_dispatched();
    vm.eval(
        r#"
        (() => {
          globalThis.__wasmDeclaredLimitResults = [];
          const memory64Maximum = new Uint8Array([
            0,97,115,109,1,0,0,0,
            5,10,1,5,0,128,128,128,128,128,128,64
          ]);
          const memory64Initial = new Uint8Array([
            0,97,115,109,1,0,0,0,
            5,9,1,4,128,128,128,128,128,128,64
          ]);
          const table32Initial = new Uint8Array([
            0,97,115,109,1,0,0,0,
            4,8,1,112,0,255,255,255,255,15
          ]);
          const record = value => __wasmDeclaredLimitResults.push(value);
          record("shape:" + [
            WebAssembly.Module.name,
            WebAssembly.Module.length,
            Object.prototype.propertyIsEnumerable.call(WebAssembly, "Module"),
            WebAssembly.validate.name,
            WebAssembly.validate.length,
            Object.prototype.propertyIsEnumerable.call(WebAssembly, "validate"),
            WebAssembly.compile.name,
            WebAssembly.compile.length,
            Object.prototype.propertyIsEnumerable.call(WebAssembly, "compile"),
            WebAssembly.instantiate.name,
            WebAssembly.instantiate.length,
            Object.prototype.propertyIsEnumerable.call(WebAssembly, "instantiate"),
            WebAssembly.Instance.name,
            WebAssembly.Instance.length
          ].join(":"));
          record("validate:" + [
            WebAssembly.validate(memory64Maximum),
            WebAssembly.validate(memory64Initial),
            WebAssembly.validate(table32Initial)
          ].join(":"));
          record("module-statics:" + ["exports", "imports", "customSections"]
            .flatMap(name => [
              typeof WebAssembly.Module[name],
              WebAssembly.Module[name] && WebAssembly.Module[name].name,
              WebAssembly.Module[name] && WebAssembly.Module[name].length,
              Object.prototype.propertyIsEnumerable.call(WebAssembly.Module, name)
            ]).join(":"));

          const directMaximumModule = new WebAssembly.Module(memory64Maximum);
          const directMaximumInstance = new WebAssembly.Instance(directMaximumModule);
          record("direct-maximum:" + [
            directMaximumModule instanceof WebAssembly.Module,
            directMaximumInstance instanceof WebAssembly.Instance
          ].join(":"));
          const directInitialModule = new WebAssembly.Module(memory64Initial);
          record("direct-initial-module:" +
            (directInitialModule instanceof WebAssembly.Module));
          try {
            new WebAssembly.Instance(directInitialModule);
            record("direct-initial-sync:resolved");
          } catch (error) {
            record("direct-initial-sync:" + error.constructor.name);
          }
          try {
            new WebAssembly.Instance(structuredClone(directInitialModule));
            record("direct-initial-clone:resolved");
          } catch (error) {
            record("direct-initial-clone:" + error.constructor.name);
          }

          const guarded = (label, bytes) => WebAssembly.compile(bytes).then(module => {
            record(label + "-module:" + (module instanceof WebAssembly.Module));
            try {
              new WebAssembly.Instance(module);
              record(label + "-sync:resolved");
            } catch (error) {
              record(label + "-sync:" + error.constructor.name);
            }
            const clone = structuredClone(module);
            try {
              new WebAssembly.Instance(clone);
              record(label + "-clone:resolved");
            } catch (error) {
              record(label + "-clone:" + error.constructor.name);
            }
            return WebAssembly.instantiate(module).then(
              () => record(label + "-async:resolved"),
              error => record(label + "-async:" + error.constructor.name)
            );
          });

          Promise.all([
            WebAssembly.compile(memory64Maximum).then(module =>
              WebAssembly.instantiate(module).then(instance => {
                record("maximum:" + [
                  module instanceof WebAssembly.Module,
                  instance instanceof WebAssembly.Instance
                ].join(":"));
              })
            ),
            guarded("memory-initial", memory64Initial),
            guarded("table-initial", table32Initial)
          ]).then(
            () => record("done"),
            error => record("unexpected:" + error.constructor.name + ":" + error.message)
          );
          return "scheduled";
        })()
        "#,
    )
    .expect("WebAssembly declared-limit setup should evaluate");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if vm
            .eval("__wasmDeclaredLimitResults.includes('done')")
            .expect("WebAssembly declared-limit completion should evaluate")
            == "true"
        {
            break;
        }

        if vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("production selected Page-task dispatcher should run")
        {
            continue;
        }

        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .expect("WebAssembly declared-limit foreground task should settle before timeout");
        let arrived = tokio::time::timeout(remaining, vm.wait_for_v8_foreground_task_arrival())
            .await
            .expect("WebAssembly declared-limit foreground task should settle before timeout");
        assert!(
            arrived,
            "V8 foreground source closed before the declared-limit task arrived"
        );
    }
    let result = vm
        .eval("__wasmDeclaredLimitResults.slice().sort().join('|')")
        .expect("WebAssembly declared-limit results should evaluate");
    assert_eq!(
        result,
        "direct-initial-clone:RangeError|direct-initial-module:true|direct-initial-sync:RangeError|direct-maximum:true:true|done|maximum:true:true|memory-initial-async:RangeError|memory-initial-clone:RangeError|memory-initial-module:true|memory-initial-sync:RangeError|module-statics:function:exports:1:true:function:imports:1:true:function:customSections:2:true|shape:Module:1:false:validate:1:true:compile:1:true:instantiate:1:true:Instance:1|table-initial-async:RangeError|table-initial-clone:RangeError|table-initial-module:true|table-initial-sync:RangeError|validate:true:true:true"
    );
}
#[test]
fn webassembly_streaming_apis_compile_and_instantiate_response_bodies() {
    let mut vm = new_storage_test_vm("https://wasm-streaming.test/");
    vm.eval(
        r#"
        (() => {
          globalThis.__wasmStreamingResults = [];
          const bytes = new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);
          const compileResponse = new Response(bytes, {
            headers: { "Content-Type": "APPLICATION/WASM" }
          });
          WebAssembly.compileStreaming(compileResponse).then(
            module => __wasmStreamingResults.push([
              "compile",
              module instanceof WebAssembly.Module,
              WebAssembly.compileStreaming.length,
              Object.prototype.propertyIsEnumerable.call(WebAssembly, "compileStreaming")
            ].join(":")),
            error => __wasmStreamingResults.push("compile-error:" + error.constructor.name)
          );

          const instantiateResponse = new Response(bytes, {
            headers: { "Content-Type": "application/wasm" }
          });
          WebAssembly.instantiateStreaming(instantiateResponse).then(
            result => __wasmStreamingResults.push([
              "instantiate",
              result.module instanceof WebAssembly.Module,
              result.instance instanceof WebAssembly.Instance,
              WebAssembly.instantiateStreaming.length,
              Object.prototype.propertyIsEnumerable.call(WebAssembly, "instantiateStreaming")
            ].join(":")),
            error => __wasmStreamingResults.push("instantiate-error:" + error.constructor.name)
          );

          const parameterizedMime = new Response(bytes, {
            headers: { "Content-Type": "application/wasm;charset=UTF-8" }
          });
          WebAssembly.compileStreaming(parameterizedMime).then(
            () => __wasmStreamingResults.push("parameterized:resolved"),
            error => __wasmStreamingResults.push("parameterized-error:" + error.constructor.name)
          );

          const trailingSemicolonMime = new Response(bytes, {
            headers: { "Content-Type": "application/wasm;" }
          });
          WebAssembly.instantiateStreaming(trailingSemicolonMime).then(
            () => __wasmStreamingResults.push("trailing-semicolon:resolved"),
            error => __wasmStreamingResults.push("trailing-semicolon-error:" + error.constructor.name)
          );

          const malformedParameterMime = new Response(bytes, {
            headers: { "Content-Type": "application/wasm;x" }
          });
          WebAssembly.compileStreaming(malformedParameterMime).then(
            () => __wasmStreamingResults.push("malformed-parameter:resolved"),
            error => __wasmStreamingResults.push("malformed-parameter-error:" + error.constructor.name)
          );
          return "scheduled";
        })()
        "#,
    )
    .expect("WebAssembly streaming API setup should evaluate");

    for _ in 0..6 {
        vm.eval("0")
            .expect("WebAssembly streaming promise microtasks should drain");
    }
    let result = vm
        .eval("__wasmStreamingResults.sort().join('|')")
        .expect("WebAssembly streaming results should evaluate");
    assert_eq!(
        result,
        "compile:true:1:true|instantiate:true:true:1:true|malformed-parameter:resolved|parameterized:resolved|trailing-semicolon:resolved"
    );
}
#[test]
fn webassembly_streaming_apis_reject_invalid_response_inputs() {
    let mut vm = new_storage_test_vm("https://wasm-streaming-errors.test/");
    vm.eval(
        r#"
        (() => {
          globalThis.__wasmStreamingErrors = [];
          const bytes = new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);

          const wrongMime = new Response(bytes, {
            headers: { "Content-Type": "text/plain; charset=UTF-8" }
          });
          WebAssembly.compileStreaming(wrongMime).then(
            () => __wasmStreamingErrors.push("wrong-mime:resolved"),
            error => __wasmStreamingErrors.push("wrong-mime:" + error.constructor.name)
          );

          const used = new Response(bytes, {
            headers: { "Content-Type": "application/wasm" }
          });
          used.arrayBuffer();
          WebAssembly.instantiateStreaming(used).then(
            () => __wasmStreamingErrors.push("used:resolved"),
            error => __wasmStreamingErrors.push("used:" + error.constructor.name)
          );

          WebAssembly.compileStreaming({}).then(
            () => __wasmStreamingErrors.push("object:resolved"),
            error => __wasmStreamingErrors.push("object:" + error.constructor.name)
          );

          const fakeResponse = {
            ok: true,
            body: null,
            bodyUsed: false,
            headers: { get: () => "application/wasm" },
            arrayBuffer: () => Promise.resolve(bytes.buffer),
            [Symbol.toStringTag]: "Response"
          };
          WebAssembly.compileStreaming(fakeResponse).then(
            () => __wasmStreamingErrors.push("fake-response:resolved"),
            error => __wasmStreamingErrors.push("fake-response:" + error.constructor.name)
          );
          WebAssembly.instantiateStreaming(Promise.resolve(fakeResponse), {}).then(
            () => __wasmStreamingErrors.push("fake-response-promise:resolved"),
            error => __wasmStreamingErrors.push("fake-response-promise:" + error.constructor.name)
          );
          return "scheduled";
        })()
        "#,
    )
    .expect("WebAssembly streaming error setup should evaluate");

    for _ in 0..6 {
        vm.eval("0")
            .expect("WebAssembly streaming rejection microtasks should drain");
    }
    let result = vm
        .eval("__wasmStreamingErrors.sort().join('|')")
        .expect("WebAssembly streaming rejection results should evaluate");
    assert_eq!(
        result,
        "fake-response-promise:TypeError|fake-response:TypeError|object:TypeError|used:TypeError|wrong-mime:TypeError"
    );
}
#[test]
fn webassembly_streaming_apis_use_response_internal_slots() {
    let mut vm = new_storage_test_vm("https://wasm-streaming-slots.test/");
    vm.eval(
        r#"
        (() => {
          globalThis.__wasmStreamingSlotResults = [];
          const bytes = new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);

          function patchPageVisibleResponseSurface(response, headerValue, bodyUsedValue) {
            Object.defineProperty(response, "arrayBuffer", {
              configurable: true,
              value() {
                return Promise.reject(new Error("patched-arrayBuffer"));
              }
            });
            Object.defineProperty(response.headers, "get", {
              configurable: true,
              value() {
                return headerValue;
              }
            });
            Object.defineProperty(response, "bodyUsed", {
              configurable: true,
              get() {
                return bodyUsedValue;
              }
            });
          }

          const compileResponse = new Response(bytes, {
            headers: { "Content-Type": "application/wasm" }
          });
          patchPageVisibleResponseSurface(compileResponse, "application/wasm", false);
          WebAssembly.compileStreaming(compileResponse).then(
            module => __wasmStreamingSlotResults.push("compile:" + (module instanceof WebAssembly.Module)),
            error => __wasmStreamingSlotResults.push("compile-error:" + error.message)
          );

          const instantiateResponse = new Response(bytes, {
            headers: { "Content-Type": "application/wasm" }
          });
          patchPageVisibleResponseSurface(instantiateResponse, "text/plain", true);
          WebAssembly.instantiateStreaming(instantiateResponse, {}).then(
            result => __wasmStreamingSlotResults.push([
              "instantiate",
              result.module instanceof WebAssembly.Module,
              result.instance instanceof WebAssembly.Instance
            ].join(":")),
            error => __wasmStreamingSlotResults.push("instantiate-error:" + error.message)
          );

          return "scheduled";
        })()
        "#,
    )
    .expect("WebAssembly streaming internal slot setup should evaluate");

    for _ in 0..6 {
        vm.eval("0")
            .expect("WebAssembly streaming internal slot microtasks should drain");
    }
    let result = vm
        .eval("__wasmStreamingSlotResults.sort().join('|')")
        .expect("WebAssembly streaming internal slot results should evaluate");
    assert_eq!(result, "compile:true|instantiate:true:true");
}
#[test]
fn webassembly_runtime_exposes_type_reflection_surface() {
    let mut vm = new_storage_test_vm("https://wasm-type-reflection.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const throwsName = callback => {
                try {
                  callback();
                  return "none";
                } catch (error) {
                  return error && error.name;
                }
              };

              const memory = new WebAssembly.Memory({ minimum: 2, maximum: 4 });
              const memoryType = memory.type();
              const table = new WebAssembly.Table({ element: "funcref", minimum: 1, maximum: 3 });
              const tableType = table.type();
              const global = new WebAssembly.Global({ value: "funcref", mutable: false });
              const globalType = global.type();
              const mutableGlobal = new WebAssembly.Global({ value: "i32", mutable: true });
              const globalValueSetter = Object.getOwnPropertyDescriptor(WebAssembly.Global.prototype, "value").set;
              const missingSetterResult = globalValueSetter.call(mutableGlobal);
              const globalConstructorOrder = [];
              new WebAssembly.Global({
                get value() {
                  globalConstructorOrder.push("descriptor value");
                  return {
                    toString() {
                      globalConstructorOrder.push("descriptor value toString");
                      return "f64";
                    }
                  };
                },
                get mutable() {
                  globalConstructorOrder.push("descriptor mutable");
                  return false;
                }
              }, {
                valueOf() {
                  globalConstructorOrder.push("value valueOf()");
                }
              });
              const arrayFrom = Array.from;
              Array.from = () => { throw new Error("WebAssembly descriptor reflection must not use Array.from"); };
              const tag = new WebAssembly.Tag({
                parameters: { 0: "i32", 1: "i64", length: 2 }
              });
              Array.from = arrayFrom;
              const tagType = tag.type();
              const exception = new WebAssembly.Exception(tag, [7, 9n]);
              const fakeMemory = { buffer: { byteLength: 65536 } };
              Object.setPrototypeOf(fakeMemory, WebAssembly.Memory.prototype);
              const fakeTable = { length: 1 };
              Object.setPrototypeOf(fakeTable, WebAssembly.Table.prototype);
              const namespaceInstanceDescriptor =
                Object.getOwnPropertyDescriptor(WebAssembly, "namespaceInstance");

              return JSON.stringify({
                memoryTypeMinimum: memoryType.minimum,
                memoryTypeMaximum: memoryType.maximum,
                memoryBothBounds: throwsName(() => new WebAssembly.Memory({ initial: 1, minimum: 1 })),
                memoryPlainReceiver: throwsName(() => WebAssembly.Memory.prototype.type.call({ buffer: { byteLength: 65536 } })),
                memorySpoofedReceiver: throwsName(() => WebAssembly.Memory.prototype.type.call(fakeMemory)),
                tableTypeMinimum: tableType.minimum,
                tableTypeMaximum: tableType.maximum,
                tableTypeElement: tableType.element,
                tableBothBounds: throwsName(() => new WebAssembly.Table({ element: "anyfunc", initial: 0, minimum: 0 })),
                tablePlainReceiver: throwsName(() => WebAssembly.Table.prototype.type.call({ length: 1 })),
                tableSpoofedReceiver: throwsName(() => WebAssembly.Table.prototype.type.call(fakeTable)),
                globalTypeMutable: globalType.mutable,
                globalTypeValue: globalType.value,
                globalTypeKeys: Object.getOwnPropertyNames(globalType),
                globalValueSetterName: globalValueSetter.name,
                globalValueSetterLength: globalValueSetter.length,
                missingSetterResult,
                missingSetterValue: mutableGlobal.value,
                globalConstructorOrder: globalConstructorOrder.join("|"),
                wasmFunction: typeof WebAssembly.Function,
                tagType,
                exceptionLength: WebAssembly.Exception.length,
                exceptionOutOfRange: throwsName(() => exception.getArg(tag, 2)),
                exceptionValue: String(exception.getArg(tag, 1)),
                namespaceInstanceName: WebAssembly.namespaceInstance.name,
                namespaceInstanceLength: WebAssembly.namespaceInstance.length,
                namespaceInstanceEnumerable: namespaceInstanceDescriptor.enumerable,
                namespaceInstanceWritable: namespaceInstanceDescriptor.writable,
                namespaceInstanceConfigurable: namespaceInstanceDescriptor.configurable,
                namespaceInstancePlainObject: throwsName(() => WebAssembly.namespaceInstance({})),
                namespaceInstanceMissing: throwsName(() => WebAssembly.namespaceInstance())
              });
            })()
            "#,
        )
        .expect("WebAssembly type reflection probe should evaluate");

    assert_eq!(
        result,
        r#"{"memoryTypeMinimum":2,"memoryTypeMaximum":4,"memoryBothBounds":"TypeError","memoryPlainReceiver":"TypeError","memorySpoofedReceiver":"TypeError","tableTypeMinimum":1,"tableTypeMaximum":3,"tableTypeElement":"funcref","tableBothBounds":"TypeError","tablePlainReceiver":"TypeError","tableSpoofedReceiver":"TypeError","globalTypeMutable":false,"globalTypeValue":"funcref","globalTypeKeys":["mutable","value"],"globalValueSetterName":"set value","globalValueSetterLength":1,"missingSetterValue":0,"globalConstructorOrder":"descriptor mutable|descriptor value|descriptor value toString|value valueOf()","wasmFunction":"undefined","tagType":{"parameters":["i32","i64"]},"exceptionLength":2,"exceptionOutOfRange":"RangeError","exceptionValue":"9","namespaceInstanceName":"namespaceInstance","namespaceInstanceLength":1,"namespaceInstanceEnumerable":false,"namespaceInstanceWritable":true,"namespaceInstanceConfigurable":true,"namespaceInstancePlainObject":"TypeError","namespaceInstanceMissing":"TypeError"}"#
    );
}
#[test]
fn zhihu_probe_navigator_shape_matches_chromium_branding() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const proto = Object.getPrototypeOf(navigator);
              const ua = Object.getOwnPropertyDescriptor(proto, "userAgent");
              const webdriver = Object.getOwnPropertyDescriptor(proto, "webdriver");
              const languages = Object.getOwnPropertyDescriptor(proto, "languages");
              return JSON.stringify({
                ctorType: typeof Navigator,
                ctorName: navigator.constructor && navigator.constructor.name,
                tag: Object.prototype.toString.call(navigator),
                ownUserAgent: Object.prototype.hasOwnProperty.call(navigator, "userAgent"),
                ownWebdriver: Object.prototype.hasOwnProperty.call(navigator, "webdriver"),
                protoCtor: proto.constructor && proto.constructor.name,
                uaGetterType: typeof ua?.get,
                webdriverGetterType: typeof webdriver?.get,
                languagesGetterType: typeof languages?.get,
                instanceofNavigator: navigator instanceof Navigator
              });
            })()
            "#,
        )
        .expect("navigator branding probe should evaluate");

    assert_eq!(
        result,
        r#"{"ctorType":"function","ctorName":"Navigator","tag":"[object Navigator]","ownUserAgent":false,"ownWebdriver":false,"protoCtor":"Navigator","uaGetterType":"function","webdriverGetterType":"function","languagesGetterType":"function","instanceofNavigator":true}"#
    );
}
#[test]
fn async_clipboard_interfaces_are_branded_and_round_trip_text_data() {
    let mut vm = new_storage_test_vm("https://async-clipboard.test/");

    let shape = vm
        .eval(
            r#"
            (() => {
              const clipboard = navigator.clipboard;
              const item = new ClipboardItem(
                { "text/plain": "hello", "not a/real type": "opaque" },
                { presentationStyle: "inline" }
              );
              const writableItem = new ClipboardItem({ "text/plain": "hello" });
              const invalidCustomItem = new ClipboardItem({
                "application/x-private": new Blob(["x"], {
                  type: "application/x-private"
                })
              });
              const mismatchedCustomItem = new ClipboardItem({
                "web text/plain": new Blob(["x"], { type: "text/custom" })
              });
              const stringPngItem = new ClipboardItem({ "image/png": "not an image" });
              const method = (object, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(object, name);
                return [
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":");
              };
              const accessor = (object, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(object, name);
                return [
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.get?.length,
                  typeof descriptor?.set,
                  descriptor?.enumerable,
                  descriptor?.configurable
                ].join(":");
              };
              const errorName = callback => {
                try {
                  callback();
                  return "none";
                } catch (error) {
                  return error && error.name;
                }
              };
              const presentationStyleGetter = Object.getOwnPropertyDescriptor(
                ClipboardItem.prototype,
                "presentationStyle"
              ).get;
              const typesGetter = Object.getOwnPropertyDescriptor(
                ClipboardItem.prototype,
                "types"
              ).get;

              globalThis.__asyncClipboardProbe = { state: "pending" };
              const rejectionName = promise => Promise.resolve(promise).then(
                () => "resolved",
                error => error && error.name
              );
              (async () => {
                const initialBlob = await item.getType("text/plain");
                const initialText = await initialBlob.text();
                const opaqueBlob = await item.getType("not a/real type");
                const opaqueText = await opaqueBlob.text();
                await clipboard.write([writableItem]);
                const writtenText = await clipboard.readText();
                const readItems = await clipboard.read({ unsanitized: ["text/html"] });
                await clipboard.writeText("next");
                const textItems = await clipboard.read();
                const textBlob = await textItems[0].getType("text/plain");
                const roundTripText = await textBlob.text();
                const rejections = await Promise.all([
                  rejectionName(Clipboard.prototype.read.call({})),
                  rejectionName(Clipboard.prototype.write.call({}, [item])),
                  rejectionName(clipboard.writeText()),
                  rejectionName(clipboard.write([])),
                  rejectionName(clipboard.read({ unsanitized: ["text/html", "text/plain"] })),
                  rejectionName(item.getType()),
                  rejectionName(item.getType("missing/type")),
                  rejectionName(ClipboardItem.prototype.getType.call({}, "text/plain")),
                  rejectionName(clipboard.write([invalidCustomItem])),
                  rejectionName(clipboard.write([mismatchedCustomItem])),
                  rejectionName(clipboard.write([stringPngItem]))
                ]);
                globalThis.__asyncClipboardProbe = {
                  state: "done",
                  initialBlob: [initialBlob instanceof Blob, initialBlob.type, initialText],
                  writtenText,
                  read: [
                    readItems.length,
                    readItems[0] === writableItem,
                    opaqueText,
                    Object.isFrozen(readItems)
                  ],
                  textWrite: [
                    textItems.length,
                    textItems[0] instanceof ClipboardItem,
                    textItems[0].types.join(","),
                    roundTripText
                  ],
                  rejections
                };
              })().catch(error => {
                globalThis.__asyncClipboardProbe = {
                  state: "failed",
                  error: `${error && error.name}:${error && error.message}`
                };
              });

              return JSON.stringify({
                constructors: [
                  typeof Clipboard,
                  Clipboard.name,
                  Clipboard.length,
                  typeof ClipboardItem,
                  ClipboardItem.name,
                  ClipboardItem.length
                ],
                clipboard: [
                  navigator.clipboard === navigator.clipboard,
                  clipboard instanceof Clipboard,
                  clipboard instanceof EventTarget,
                  Object.getPrototypeOf(clipboard) === Clipboard.prototype,
                  Object.prototype.toString.call(clipboard),
                  Object.keys(clipboard).join(","),
                  Object.keys(Clipboard.prototype).join(",")
                ],
                methods: [
                  method(Clipboard.prototype, "read"),
                  method(Clipboard.prototype, "readText"),
                  method(Clipboard.prototype, "write"),
                  method(Clipboard.prototype, "writeText")
                ],
                item: [
                  item instanceof ClipboardItem,
                  Object.getPrototypeOf(item) === ClipboardItem.prototype,
                  Object.prototype.toString.call(item),
                  Object.keys(item).join(","),
                  item.presentationStyle,
                  item.types.join(","),
                  Object.isFrozen(item.types),
                  Object.keys(ClipboardItem.prototype).join(",")
                ],
                itemMembers: [
                  accessor(ClipboardItem.prototype, "presentationStyle"),
                  accessor(ClipboardItem.prototype, "types"),
                  method(ClipboardItem.prototype, "getType"),
                  method(ClipboardItem, "supports")
                ],
                supports: [
                  ClipboardItem.supports("text/plain"),
                  ClipboardItem.supports("web foo/bar"),
                  ClipboardItem.supports("foo/bar")
                ],
                syncErrors: [
                  errorName(() => new Clipboard()),
                  errorName(() => Clipboard()),
                  errorName(() => ClipboardItem({ "text/plain": "x" })),
                  errorName(() => new ClipboardItem()),
                  errorName(() => new ClipboardItem(null)),
                  errorName(() => new ClipboardItem({})),
                  errorName(() => presentationStyleGetter.call({})),
                  errorName(() => typesGetter.call({}))
                ]
              });
            })()
            "#,
        )
        .expect("Async Clipboard interface shape should evaluate");

    assert_eq!(
        shape,
        r#"{"constructors":["function","Clipboard",0,"function","ClipboardItem",1],"clipboard":[true,true,true,true,"[object Clipboard]","","read,readText,write,writeText"],"methods":["function:read:0:true:true:true","function:readText:0:true:true:true","function:write:1:true:true:true","function:writeText:1:true:true:true"],"item":[true,true,"[object ClipboardItem]","","inline","text/plain,not a/real type",true,"presentationStyle,types,getType"],"itemMembers":["function:get presentationStyle:0:undefined:true:true","function:get types:0:undefined:true:true","function:getType:1:true:true:true","function:supports:1:true:true:true"],"supports":[true,true,false],"syncErrors":["TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError"]}"#
    );

    vm.eval("0")
        .expect("Async Clipboard promise operations should drain");
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__asyncClipboardProbe)")
            .expect("Async Clipboard promise result should evaluate"),
        r#"{"state":"done","initialBlob":[true,"text/plain","hello"],"writtenText":"hello","read":[1,false,"opaque",false],"textWrite":[1,true,"text/plain","next"],"rejections":["TypeError","TypeError","TypeError","NotAllowedError","NotAllowedError","TypeError","NotFoundError","TypeError","NotAllowedError","NotAllowedError","TypeError"]}"#
    );
}
#[test]
fn zhihu_probe_navigator_plugin_and_mime_surfaces_match_chromium_pdf_builtins() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              return JSON.stringify({
                languages: Array.from(navigator.languages || []),
                hardwareConcurrencyPositive: navigator.hardwareConcurrency > 0,
                pluginsCtorType: typeof PluginArray,
                pluginsCtorName: navigator.plugins && navigator.plugins.constructor && navigator.plugins.constructor.name,
                pluginsTag: Object.prototype.toString.call(navigator.plugins),
                pluginsLength: navigator.plugins.length,
                pluginsInstanceof: navigator.plugins instanceof PluginArray,
                plugin0CtorName: navigator.plugins[0] && navigator.plugins[0].constructor && navigator.plugins[0].constructor.name,
                plugin0Instanceof: navigator.plugins[0] instanceof Plugin,
                plugin0Name: navigator.plugins[0] && navigator.plugins[0].name,
                pluginNamedItem: navigator.plugins.namedItem("PDF Viewer") && navigator.plugins.namedItem("PDF Viewer").name,
                mimeTypesCtorType: typeof MimeTypeArray,
                mimeTypesCtorName: navigator.mimeTypes && navigator.mimeTypes.constructor && navigator.mimeTypes.constructor.name,
                mimeTypesTag: Object.prototype.toString.call(navigator.mimeTypes),
                mimeTypesLength: navigator.mimeTypes.length,
                mimeTypesInstanceof: navigator.mimeTypes instanceof MimeTypeArray,
                mime0CtorName: navigator.mimeTypes[0] && navigator.mimeTypes[0].constructor && navigator.mimeTypes[0].constructor.name,
                mime0Instanceof: navigator.mimeTypes[0] instanceof MimeType,
                mime0Type: navigator.mimeTypes[0] && navigator.mimeTypes[0].type,
                mime0EnabledPluginIsFirst:
                    navigator.mimeTypes[0].enabledPlugin === navigator.plugins[0],
                mime1EnabledPluginIsFirst:
                    navigator.mimeTypes[1].enabledPlugin === navigator.plugins[0],
                mimeNamedItem: navigator.mimeTypes.namedItem("application/pdf") && navigator.mimeTypes.namedItem("application/pdf").type,
                pdfViewerEnabled: navigator.pdfViewerEnabled
              });
            })()
            "#,
        )
        .expect("plugin/mime probe should evaluate");

    assert_eq!(
        result,
        r#"{"languages":["en-US","en"],"hardwareConcurrencyPositive":true,"pluginsCtorType":"function","pluginsCtorName":"PluginArray","pluginsTag":"[object PluginArray]","pluginsLength":5,"pluginsInstanceof":true,"plugin0CtorName":"Plugin","plugin0Instanceof":true,"plugin0Name":"PDF Viewer","pluginNamedItem":"PDF Viewer","mimeTypesCtorType":"function","mimeTypesCtorName":"MimeTypeArray","mimeTypesTag":"[object MimeTypeArray]","mimeTypesLength":2,"mimeTypesInstanceof":true,"mime0CtorName":"MimeType","mime0Instanceof":true,"mime0Type":"application/pdf","mime0EnabledPluginIsFirst":true,"mime1EnabledPluginIsFirst":true,"mimeNamedItem":"application/pdf","pdfViewerEnabled":true}"#
    );
}
#[test]
fn zhihu_probe_screen_shape_matches_chromium_branding() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const proto = Object.getPrototypeOf(screen);
              const availWidth = Object.getOwnPropertyDescriptor(proto, "availWidth");
              const availLeft = Object.getOwnPropertyDescriptor(proto, "availLeft");
              const orientation = Object.getOwnPropertyDescriptor(proto, "orientation");
              const descriptorSummary = name => {
                const descriptor = Object.getOwnPropertyDescriptor(proto, name);
                return `${name}:${typeof descriptor?.get}:${descriptor?.get?.name}:${descriptor?.get?.length}:${typeof descriptor?.set}:${descriptor?.enumerable}:${descriptor?.configurable}:${Object.hasOwn(screen, name)}`;
              };
              const screenDescriptorNames = [
                "availWidth",
                "availHeight",
                "availLeft",
                "availTop",
                "width",
                "height",
                "colorDepth",
                "pixelDepth",
                "orientation"
              ];
              return JSON.stringify({
                ctorType: typeof Screen,
                ctorName: screen.constructor && screen.constructor.name,
                tag: Object.prototype.toString.call(screen),
                ownAvailWidth: Object.prototype.hasOwnProperty.call(screen, "availWidth"),
                ownAvailLeft: Object.prototype.hasOwnProperty.call(screen, "availLeft"),
                ownOrientation: Object.prototype.hasOwnProperty.call(screen, "orientation"),
                internalNames: Object.getOwnPropertyNames(screen)
                  .filter(name => name.startsWith("__moliScreen"))
                  .sort(),
                protoCtor: proto.constructor && proto.constructor.name,
                availWidthGetterType: typeof availWidth?.get,
                availLeftGetterType: typeof availLeft?.get,
                orientationGetterType: typeof orientation?.get,
                descriptors: screenDescriptorNames.map(descriptorSummary),
                addEventListenerType: typeof screen.addEventListener,
                ownAddEventListener: Object.prototype.hasOwnProperty.call(screen, "addEventListener"),
                instanceofScreen: screen instanceof Screen
              });
            })()
            "#,
        )
        .expect("screen branding probe should evaluate");

    assert_eq!(
        result,
        r#"{"ctorType":"function","ctorName":"Screen","tag":"[object Screen]","ownAvailWidth":false,"ownAvailLeft":false,"ownOrientation":false,"internalNames":[],"protoCtor":"Screen","availWidthGetterType":"function","availLeftGetterType":"function","orientationGetterType":"function","descriptors":["availWidth:function:get availWidth:0:undefined:true:true:false","availHeight:function:get availHeight:0:undefined:true:true:false","availLeft:function:get availLeft:0:undefined:true:true:false","availTop:function:get availTop:0:undefined:true:true:false","width:function:get width:0:undefined:true:true:false","height:function:get height:0:undefined:true:true:false","colorDepth:function:get colorDepth:0:undefined:true:true:false","pixelDepth:function:get pixelDepth:0:undefined:true:true:false","orientation:function:get orientation:0:undefined:true:true:false"],"addEventListenerType":"function","ownAddEventListener":false,"instanceofScreen":true}"#
    );
}
#[test]
fn zhihu_probe_screen_values_match_successful_chromium_profile() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            JSON.stringify({
              width: screen.width,
              height: screen.height,
              availWidth: screen.availWidth,
              availHeight: screen.availHeight,
              availLeft: screen.availLeft,
              availTop: screen.availTop,
              colorDepth: screen.colorDepth,
              pixelDepth: screen.pixelDepth
            })
            "#,
        )
        .expect("screen value probe should evaluate");

    assert_eq!(
        result,
        r#"{"width":1920,"height":1080,"availWidth":1920,"availHeight":1080,"availLeft":0,"availTop":0,"colorDepth":24,"pixelDepth":24}"#
    );
}
#[test]
fn zhihu_probe_touch_surface_matches_successful_chromium_profile() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.createElement("div");
              const touch = new Touch({
                identifier: 7,
                target,
                clientX: 11,
                clientY: 12
              });
              const event = new TouchEvent("touchstart", {
                touches: [touch],
                targetTouches: [touch],
                changedTouches: [touch]
              });
              const touchProto = Object.getPrototypeOf(touch);
              const touchListProto = Object.getPrototypeOf(event.touches);
              const touchEventProto = Object.getPrototypeOf(event);
              const descriptorSummary = (prototype, receiver, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return `${name}:${typeof descriptor?.get}:${descriptor?.get?.name}:${descriptor?.get?.length}:${typeof descriptor?.set}:${descriptor?.enumerable}:${descriptor?.configurable}:${Object.hasOwn(receiver, name)}`;
              };
              const touchDescriptorNames = [
                "identifier",
                "target",
                "screenX",
                "screenY",
                "clientX",
                "clientY",
                "pageX",
                "pageY",
                "radiusX",
                "radiusY",
                "rotationAngle",
                "force"
              ];
              const touchEventDescriptorNames = [
                "touches",
                "targetTouches",
                "changedTouches",
                "altKey",
                "metaKey",
                "ctrlKey",
                "shiftKey"
              ];
              return JSON.stringify({
                touchType: typeof Touch,
                touchEventType: typeof TouchEvent,
                touchListType: typeof TouchList,
                touchCtorLength: Touch.length,
                touchEventCtorLength: TouchEvent.length,
                touchTag: Object.prototype.toString.call(touch),
                touchCtorName: touch.constructor && touch.constructor.name,
                touchInstanceof: touch instanceof Touch,
                touchOwnNames: Object.getOwnPropertyNames(touch),
                touchInternalNames: Object.getOwnPropertyNames(touch)
                  .filter(name => name.startsWith("__lmTouch"))
                  .sort(),
                touchProtoParentCtor: Object.getPrototypeOf(touchProto)?.constructor?.name ?? null,
                touchProtoOwnNames: Object.getOwnPropertyNames(touchProto).slice().sort(),
                touchDescriptors: touchDescriptorNames.map(name => descriptorSummary(touchProto, touch, name)),
                touchIdentifier: touch.identifier,
                touchTargetTag: touch.target && touch.target.tagName,
                touchTargetEq: touch.target === target,
                touchClientX: touch.clientX,
                touchClientY: touch.clientY,
                touchScreenX: touch.screenX,
                touchForce: touch.force,
                touchEventTag: Object.prototype.toString.call(event),
                touchEventCtorName: event.constructor && event.constructor.name,
                touchEventInstanceof: event instanceof TouchEvent,
                touchEventInstanceofUi: event instanceof UIEvent,
                touchEventInternalNames: Object.getOwnPropertyNames(event)
                  .filter(name => name.startsWith("__lmTouchEvent"))
                  .sort(),
                touchEventProtoParentCtor: Object.getPrototypeOf(touchEventProto)?.constructor?.name ?? null,
                touchEventProtoOwnNames: Object.getOwnPropertyNames(touchEventProto).slice().sort(),
                touchEventDescriptors: touchEventDescriptorNames.map(name => descriptorSummary(touchEventProto, event, name)),
                touchesTag: Object.prototype.toString.call(event.touches),
                touchesCtorName: event.touches.constructor && event.touches.constructor.name,
                touchesProtoCtor: touchListProto && touchListProto.constructor && touchListProto.constructor.name,
                touchesProtoOwnNames: Object.getOwnPropertyNames(touchListProto).slice().sort(),
                touchesLength: event.touches.length,
                targetTouchesLength: event.targetTouches.length,
                changedTouchesLength: event.changedTouches.length,
                touchItemEq: event.touches.item(0) === touch,
                touchItemMissingNull: event.touches.item(1) === null,
                altKey: event.altKey,
                metaKey: event.metaKey,
                ctrlKey: event.ctrlKey,
                shiftKey: event.shiftKey
              });
            })()
            "#,
        )
        .expect("touch probe should evaluate");

    assert_eq!(
        result,
        r#"{"touchType":"function","touchEventType":"function","touchListType":"function","touchCtorLength":1,"touchEventCtorLength":1,"touchTag":"[object Touch]","touchCtorName":"Touch","touchInstanceof":true,"touchOwnNames":[],"touchInternalNames":[],"touchProtoParentCtor":"Object","touchProtoOwnNames":["clientX","clientY","constructor","force","identifier","pageX","pageY","radiusX","radiusY","rotationAngle","screenX","screenY","target"],"touchDescriptors":["identifier:function:get identifier:0:undefined:true:true:false","target:function:get target:0:undefined:true:true:false","screenX:function:get screenX:0:undefined:true:true:false","screenY:function:get screenY:0:undefined:true:true:false","clientX:function:get clientX:0:undefined:true:true:false","clientY:function:get clientY:0:undefined:true:true:false","pageX:function:get pageX:0:undefined:true:true:false","pageY:function:get pageY:0:undefined:true:true:false","radiusX:function:get radiusX:0:undefined:true:true:false","radiusY:function:get radiusY:0:undefined:true:true:false","rotationAngle:function:get rotationAngle:0:undefined:true:true:false","force:function:get force:0:undefined:true:true:false"],"touchIdentifier":7,"touchTargetTag":"DIV","touchTargetEq":true,"touchClientX":11,"touchClientY":12,"touchScreenX":0,"touchForce":0,"touchEventTag":"[object TouchEvent]","touchEventCtorName":"TouchEvent","touchEventInstanceof":true,"touchEventInstanceofUi":true,"touchEventInternalNames":[],"touchEventProtoParentCtor":"UIEvent","touchEventProtoOwnNames":["altKey","changedTouches","constructor","ctrlKey","metaKey","shiftKey","targetTouches","touches"],"touchEventDescriptors":["touches:function:get touches:0:undefined:true:true:false","targetTouches:function:get targetTouches:0:undefined:true:true:false","changedTouches:function:get changedTouches:0:undefined:true:true:false","altKey:function:get altKey:0:undefined:true:true:false","metaKey:function:get metaKey:0:undefined:true:true:false","ctrlKey:function:get ctrlKey:0:undefined:true:true:false","shiftKey:function:get shiftKey:0:undefined:true:true:false"],"touchesTag":"[object TouchList]","touchesCtorName":"TouchList","touchesProtoCtor":"TouchList","touchesProtoOwnNames":["constructor","item","length"],"touchesLength":1,"targetTouchesLength":1,"changedTouchesLength":1,"touchItemEq":true,"touchItemMissingNull":true,"altKey":false,"metaKey":false,"ctrlKey":false,"shiftKey":false}"#
    );
}
#[test]
fn zhihu_probe_screen_orientation_surface_matches_chromium_headless_essentials() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const orientation = screen.orientation;
              const proto = Object.getPrototypeOf(orientation);
              const eventProto = Object.getPrototypeOf(proto);
              const summarizeAccessorDescriptor = name => {
                const descriptor = Object.getOwnPropertyDescriptor(proto, name);
                return [
                  !!descriptor,
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.get?.length,
                  typeof descriptor?.set,
                  descriptor?.set?.name ?? "",
                  descriptor?.set?.length ?? -1,
                  descriptor?.enumerable,
                  descriptor?.configurable,
                  Object.hasOwn(orientation, name)
                ].join(":");
              };
              const summarizeMethodDescriptor = name => {
                const descriptor = Object.getOwnPropertyDescriptor(proto, name);
                return [
                  !!descriptor,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":");
              };
              const onchangeBeforeType = typeof orientation.onchange;
              orientation.onchange = function handleOrientationChange() {};
              return JSON.stringify({
                ctorType: typeof ScreenOrientation,
                ctorName: orientation.constructor && orientation.constructor.name,
                tag: Object.prototype.toString.call(orientation),
                sameObject: screen.orientation === screen.orientation,
                protoCtor: proto && proto.constructor && proto.constructor.name,
                eventProtoCtor: eventProto && eventProto.constructor && eventProto.constructor.name,
                ownKeys: Object.getOwnPropertyNames(orientation),
                type: orientation.type,
                angle: orientation.angle,
                typeDescriptor: summarizeAccessorDescriptor("type"),
                angleDescriptor: summarizeAccessorDescriptor("angle"),
                onchangeBeforeType,
                onchangeType: typeof orientation.onchange,
                onchangeName: orientation.onchange && orientation.onchange.name,
                onchangeDescriptor: summarizeAccessorDescriptor("onchange"),
                lockType: typeof orientation.lock,
                lockName: orientation.lock && orientation.lock.name,
                lockLength: orientation.lock && orientation.lock.length,
                lockDescriptor: summarizeMethodDescriptor("lock"),
                unlockType: typeof orientation.unlock,
                unlockName: orientation.unlock && orientation.unlock.name,
                unlockLength: orientation.unlock && orientation.unlock.length,
                unlockDescriptor: summarizeMethodDescriptor("unlock"),
                instanceofScreenOrientation: orientation instanceof ScreenOrientation,
                instanceofEventTarget: orientation instanceof EventTarget
              });
            })()
            "#,
        )
        .expect("screen orientation probe should evaluate");

    assert_eq!(
        result,
        r#"{"ctorType":"function","ctorName":"ScreenOrientation","tag":"[object ScreenOrientation]","sameObject":true,"protoCtor":"ScreenOrientation","eventProtoCtor":"EventTarget","ownKeys":[],"type":"landscape-primary","angle":0,"typeDescriptor":"true:function:get type:0:undefined::-1:true:true:false","angleDescriptor":"true:function:get angle:0:undefined::-1:true:true:false","onchangeBeforeType":"object","onchangeType":"function","onchangeName":"handleOrientationChange","onchangeDescriptor":"true:function:get onchange:0:function:set onchange:1:true:true:false","lockType":"function","lockName":"lock","lockLength":1,"lockDescriptor":"true:function:lock:1:true:true:true","unlockType":"function","unlockName":"unlock","unlockLength":0,"unlockDescriptor":"true:function:unlock:0:true:true:true","instanceofScreenOrientation":true,"instanceofEventTarget":true}"#
    );
}
#[test]
fn zhihu_probe_location_shape_matches_chromium_branding() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const proto = Object.getPrototypeOf(location);
              const tag = Object.getOwnPropertyDescriptor(proto, Symbol.toStringTag);
              const operationNames = ["assign", "replace", "reload", "toString"];
              const operationShape = name => {
                const descriptor = Object.getOwnPropertyDescriptor(location, name);
                const value = descriptor && descriptor.value;
                const deleted = delete location[name];
                return [
                  name,
                  typeof value,
                  value && value.name,
                  value && value.length,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.writable,
                  descriptor && descriptor.configurable,
                  Object.prototype.hasOwnProperty.call(location, name),
                  deleted,
                  Object.prototype.hasOwnProperty.call(location, name)
                ].join(":");
              };
              let illegal = "missing";
              try {
                new Location();
                illegal = "constructed";
              } catch (error) {
                illegal = error && error.message;
              }
              return JSON.stringify({
                ctorType: typeof Location,
                ctorName: location.constructor && location.constructor.name,
                tag: Object.prototype.toString.call(location),
                protoCtor: proto && proto.constructor && proto.constructor.name,
                instanceofLocation: location instanceof Location,
                ownHref: Object.prototype.hasOwnProperty.call(location, "href"),
                ownAssign: Object.prototype.hasOwnProperty.call(location, "assign"),
                ownToString: Object.prototype.hasOwnProperty.call(location, "toString"),
                tagValue: tag && tag.value,
                operationKeys: Object.keys(location)
                  .filter(name => operationNames.includes(name))
                  .join(","),
                operations: operationNames.map(operationShape).join("|"),
                illegal
              });
            })()
            "#,
        )
        .expect("location branding probe should evaluate");

    assert_eq!(
        result,
        r#"{"ctorType":"function","ctorName":"Location","tag":"[object Location]","protoCtor":"Location","instanceofLocation":true,"ownHref":true,"ownAssign":true,"ownToString":true,"tagValue":"Location","operationKeys":"assign,replace,reload,toString","operations":"assign:function:assign:1:true:false:false:true:false:true|replace:function:replace:1:true:false:false:true:false:true|reload:function:reload:0:true:false:false:true:false:true|toString:function:toString:0:true:false:false:true:false:true","illegal":"Illegal constructor"}"#
    );
}
#[test]
fn zhihu_probe_native_names_match_chromium_shape() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const nativeNamed = (value, name) =>
                typeof value === "function" &&
                value.name === name &&
                new RegExp(`^function ${name}\\(`).test(String(value)) &&
                /\[native code\]/.test(String(value));
              const namedGetter = (owner, key, name) => {
                const getter = Object.getOwnPropertyDescriptor(owner, key)?.get;
                return typeof getter === "function" &&
                  getter.name === `get ${name}` &&
                  new RegExp(`^function get ${name}\\(`).test(String(getter)) &&
                  /\[native code\]/.test(String(getter));
              };
              return JSON.stringify({
                open: nativeNamed(window.open, "open"),
                print: nativeNamed(window.print, "print"),
                stop: nativeNamed(window.stop, "stop"),
                blur: nativeNamed(window.blur, "blur"),
                find: nativeNamed(window.find, "find"),
                getRandomValues: nativeNamed(crypto.getRandomValues, "getRandomValues"),
                isTypeSupported: nativeNamed(MediaSource.isTypeSupported, "isTypeSupported"),
                documentAllGetter: namedGetter(Document.prototype, "all", "all"),
                webdriverGetter: namedGetter(Navigator.prototype, "webdriver", "webdriver"),
                screenAvailWidthGetter: namedGetter(Screen.prototype, "availWidth", "availWidth")
              });
            })()
            "#,
        )
        .expect("native function naming probe should evaluate");

    assert_eq!(
        result,
        r#"{"open":true,"print":true,"stop":true,"blur":true,"find":true,"getRandomValues":true,"isTypeSupported":true,"documentAllGetter":true,"webdriverGetter":true,"screenAvailWidthGetter":true}"#
    );
}
#[test]
fn automation_override_preserves_the_native_webdriver_baseline() {
    let mut vm = new_storage_test_vm("https://native-automation.test/");
    for baseline in [false, true] {
        // Seed the browser-owned baseline, independently of Emulation state.
        vm.with_default_context_scope(|scope, _| {
            let global = scope.get_current_context().global(scope);
            let navigator_key = v8::String::new(scope, "navigator").unwrap();
            let navigator =
                v8::Local::<v8::Object>::try_from(global.get(scope, navigator_key.into()).unwrap())
                    .unwrap();
            let backing = v8::Local::<v8::Object>::try_from(
                crate::util::get_private_value(scope, navigator, "__moliNavigatorRuntimeData")
                    .unwrap(),
            )
            .unwrap();
            let key = v8::String::new(scope, "webdriver").unwrap();
            let value = v8::Boolean::new(scope, baseline);
            assert_eq!(backing.set(scope, key.into(), value.into()), Some(true));
            Ok(())
        })
        .unwrap();
        for enabled in [false, true, false] {
            vm.set_navigator_overrides_and_sync_surface(&moli_page_types::NavigatorOverrides {
                queries: moli_page_types::NavigatorQueryOverrides {
                    automation: enabled,
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
            assert_eq!(
                vm.eval("navigator.webdriver").unwrap(),
                (baseline || enabled).to_string()
            );
        }
    }
}
