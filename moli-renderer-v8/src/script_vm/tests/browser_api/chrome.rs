use super::*;

#[test]
fn window_chrome_matches_the_ordinary_chromium_page_surface() {
    let mut vm = new_storage_test_vm("https://chrome-surface.test/");

    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify({
                chromeType: typeof chrome,
                sameWindowValue: chrome === window.chrome,
                chromeKeys: Object.keys(chrome),
                chromePrototype: Object.getPrototypeOf(chrome) === Object.prototype,
                chromeDescriptor: (() => {
                    const descriptor = Object.getOwnPropertyDescriptor(window, "chrome");
                    return [
                        descriptor.value === chrome,
                        descriptor.writable,
                        descriptor.enumerable,
                        descriptor.configurable,
                    ];
                })(),
                loadTimesFunction: [
                    chrome.loadTimes.name,
                    chrome.loadTimes.length,
                    String(chrome.loadTimes),
                    (() => { try { return typeof new chrome.loadTimes(); } catch { return "throws"; } })(),
                ],
                csiFunction: [
                    chrome.csi.name,
                    chrome.csi.length,
                    String(chrome.csi),
                    (() => { try { return typeof new chrome.csi(); } catch { return "throws"; } })(),
                ],
                appKeys: Object.keys(chrome.app),
                appPrototype: Object.getPrototypeOf(chrome.app) === Object.prototype,
                frameChromeType: (() => {
                    const frame = document.createElement("iframe");
                    (document.body || document.documentElement || document).appendChild(frame);
                    return typeof frame.contentWindow.chrome;
                })(),
                appValues: [
                    chrome.app.isInstalled,
                    chrome.app.getDetails(),
                    chrome.app.getIsInstalled(),
                    chrome.app.runningState(),
                ],
                installState: chrome.app.InstallState,
                runningState: chrome.app.RunningState,
                runtimeType: typeof chrome.runtime,
            })
            "#,
        )
        .expect("inspect window.chrome"),
        r#"{"chromeType":"object","sameWindowValue":true,"chromeKeys":["loadTimes","csi","app"],"chromePrototype":true,"chromeDescriptor":[true,true,true,false],"loadTimesFunction":["",0,"function () { [native code] }","object"],"csiFunction":["",0,"function () { [native code] }","object"],"appKeys":["isInstalled","getDetails","getIsInstalled","installState","runningState","InstallState","RunningState"],"appPrototype":true,"frameChromeType":"object","appValues":[false,null,false,"cannot_run"],"installState":{"DISABLED":"disabled","INSTALLED":"installed","NOT_INSTALLED":"not_installed"},"runningState":{"CANNOT_RUN":"cannot_run","READY_TO_RUN":"ready_to_run","RUNNING":"running"},"runtimeType":"undefined"}"#
    );
}

#[test]
fn chrome_timing_snapshots_have_chromium_shape_and_ignore_assignment() {
    let mut vm = new_storage_test_vm("https://chrome-timing.test/");

    assert_eq!(
        vm.eval(
            r#"
            const loadTimes = chrome.loadTimes();
            const csi = chrome.csi();
            const requestTime = loadTimes.requestTime;
            const startE = csi.startE;
            loadTimes.requestTime = -1;
            csi.startE = -1;
            chrome.app.isInstalled = true;
            JSON.stringify({
                loadTimesKeys: Object.keys(loadTimes),
                csiKeys: Object.keys(csi),
                timingTypes: [
                    typeof loadTimes.requestTime,
                    typeof loadTimes.navigationType,
                    typeof loadTimes.wasFetchedViaSpdy,
                    typeof csi.startE,
                    typeof csi.tran,
                ],
                timingRelations: [
                    loadTimes.requestTime === requestTime,
                    loadTimes.requestTime === loadTimes.startLoadTime,
                    loadTimes.requestTime > 0,
                    csi.startE === startE,
                    csi.startE === loadTimes.requestTime * 1000,
                    csi.pageT >= 0,
                ],
                fixedValues: [
                    loadTimes.firstPaintAfterLoadTime,
                    loadTimes.navigationType,
                    loadTimes.wasFetchedViaSpdy,
                    loadTimes.wasNpnNegotiated,
                    loadTimes.npnNegotiatedProtocol,
                    loadTimes.wasAlternateProtocolAvailable,
                    loadTimes.connectionInfo,
                    csi.tran,
                    chrome.app.isInstalled,
                ],
            })
            "#,
        )
        .expect("inspect Chromium timing compatibility objects"),
        r#"{"loadTimesKeys":["requestTime","startLoadTime","commitLoadTime","finishDocumentLoadTime","finishLoadTime","firstPaintTime","firstPaintAfterLoadTime","navigationType","wasFetchedViaSpdy","wasNpnNegotiated","npnNegotiatedProtocol","wasAlternateProtocolAvailable","connectionInfo"],"csiKeys":["startE","onloadT","pageT","tran"],"timingTypes":["number","string","boolean","number","number"],"timingRelations":[true,true,true,true,true,true],"fixedValues":[0,"Other",false,false,"",false,"unknown",15,false]}"#
    );
}

#[test]
fn chrome_app_install_state_is_async_and_validates_its_callback() {
    let mut vm = new_storage_test_vm("https://chrome-app.test/");

    assert_eq!(
        vm.eval(
            r#"
            globalThis.chromeInstallStateResult = "pending";
            globalThis.chromeInstallStateOrder = [];
            let synchronous = true;
            const returned = chrome.app.installState((state) => {
                globalThis.chromeInstallStateResult = `${state}:${synchronous}`;
                chromeInstallStateOrder.push("app");
            });
            Promise.resolve().then(() => chromeInstallStateOrder.push("promise"));
            synchronous = false;
            String(returned)
            "#,
        )
        .expect("schedule chrome.app.installState callback"),
        "undefined"
    );
    assert_eq!(
        vm.eval("JSON.stringify([chromeInstallStateResult, chromeInstallStateOrder])")
            .expect("inspect chrome.app.installState before its task"),
        r#"["pending",["promise"]]"#
    );
    assert!(matches!(
        vm.run_next_timeout_for_test()
            .expect("chrome.app.installState task should run"),
        crate::host::HostTimeoutRunResult::Consumed
    ));
    assert_eq!(
        vm.eval("JSON.stringify([chromeInstallStateResult, chromeInstallStateOrder])")
            .expect("read chrome.app.installState callback result"),
        r#"["not_installed:false",["promise","app"]]"#
    );
    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify([
                chrome.app.installState() === undefined,
                (() => { try { chrome.app.getDetails(1); return false; } catch (error) { return error.message; } })(),
                (() => { try { chrome.app.installState(1); return false; } catch (error) { return error.message; } })(),
            ])
            "#,
        )
        .expect("validate chrome.app argument handling"),
        r#"[true,"Error in invocation of app.getDetails(): ","Error in invocation of app.installState(function callback): "]"#
    );
}
