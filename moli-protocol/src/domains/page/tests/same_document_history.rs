use super::*;

const SESSION: &str = "SID-COMMIT-HISTORY";
const FRAME: &str = "TID-COMMIT-HISTORY";

struct SameDocumentPage {
    ctx: TestContext,
    base_url: String,
    browser_base: usize,
    server: tokio::task::JoinHandle<()>,
    download_requests: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    download_directory: Option<std::path::PathBuf>,
}

impl Drop for SameDocumentPage {
    fn drop(&mut self) {
        self.server.abort();
        if let Some(path) = &self.download_directory {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

impl SameDocumentPage {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let download_requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_downloads = download_requests.clone();
        let server = tokio::spawn(async move {
            let app = axum::Router::new().route(
                "/history.html",
                axum::routing::get(move |uri: axum::http::Uri| {
                    let observed_downloads = observed_downloads.clone();
                    async move {
                        if uri.query().is_some_and(|query| query.contains("download=")) {
                            observed_downloads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        }
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                            "<!doctype html><title>History commits</title>",
                        )
                    }
                }),
            );
            axum::serve(listener, app).await.unwrap();
        });
        let mut ctx = TestContext::new();
        load_bc_with_session(
            &mut ctx,
            "BID-COMMIT-HISTORY",
            FRAME,
            SESSION,
            "about:blank",
        );
        ctx.enable_page_events_for_test(Some(SESSION));
        let mut page = Self {
            ctx,
            base_url: format!("http://{addr}/history.html"),
            browser_base: 0,
            server,
            download_requests,
            download_directory: None,
        };
        page.command("Page.navigate", json!({ "url": page.base_url }))
            .await;
        wait_until_frame_stopped_loading(&mut page.ctx, FRAME).await;
        let history = page.command("Page.getNavigationHistory", json!({})).await;
        page.browser_base = history["currentIndex"].as_u64().unwrap() as usize;
        page.evaluate(
            r#"
                globalThis.observations = [];
                navigation.addEventListener('currententrychange', () => {
                    observations.push([location.href, document.URL, navigation.currentEntry.url]);
                });
                for (const type of ['popstate', 'hashchange']) {
                    addEventListener(type, () => {
                        observations.push([location.href, document.URL, navigation.currentEntry.url]);
                    });
                }
                void 0;
            "#,
        ).await;
        page.ctx.sent.clear();
        page
    }

    async fn allow_downloads(&mut self) {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "moli-intercepted-download-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        self.download_directory = Some(path.clone());
        self.ctx
            .process_and_wait_for_response_async(json!({
                "id": 1002,
                "method": "Browser.setDownloadBehavior",
                "params": {
                    "behavior": "allow",
                    "downloadPath": path.to_string_lossy(),
                    "browserContextId": "BID-COMMIT-HISTORY",
                    "eventsEnabled": true,
                },
            }))
            .await;
        let response = take_response_by_id(&mut self.ctx, 1002);
        assert!(response["error"].is_null(), "{response}");
    }

    async fn command(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        self.ctx
            .process_and_wait_for_response_async(json!({
                "id": 1001,
                "method": method,
                "sessionId": SESSION,
                "params": params,
            }))
            .await;
        let response = take_response_by_id(&mut self.ctx, 1001);
        assert!(response["error"].is_null(), "{method}: {response}");
        response["result"].clone()
    }

    async fn evaluate(&mut self, expression: &str) -> serde_json::Value {
        let result = self
            .command(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                }),
            )
            .await;
        assert!(
            result["exceptionDetails"].is_null(),
            "{expression}: {result}"
        );
        result["result"]["value"].clone()
    }

    async fn run(&mut self, expression: &str, final_fragment: &str) {
        self.evaluate(expression).await;
        let expected_url = format!("{}{final_fragment}", self.base_url);
        wait_until_message(&mut self.ctx, SESSION, "same-document commit", |message| {
            message["method"] == "Page.navigatedWithinDocument"
                && message["params"]["url"] == expected_url
        })
        .await;
        self.evaluate("new Promise(resolve => setTimeout(resolve, 0))")
            .await;
    }

    async fn assert_history(&mut self, fragments: &[&str], index: usize) {
        let urls = fragments
            .iter()
            .map(|fragment| format!("{}{fragment}", self.base_url))
            .collect::<Vec<_>>();
        let renderer = self
            .evaluate(
                r#"({
            urls: navigation.entries().map(entry => entry.url),
            index: navigation.currentEntry.index,
            href: location.href,
            documentURL: document.URL,
            observations,
        })"#,
            )
            .await;
        assert_eq!(renderer["urls"], json!(urls));
        assert_eq!(renderer["index"], json!(index));
        assert_eq!(renderer["href"], json!(urls[index]));
        assert_eq!(renderer["documentURL"], renderer["href"]);
        for observation in renderer["observations"].as_array().unwrap() {
            assert_eq!(
                observation[0], observation[1],
                "Document.URL during callback"
            );
            assert_eq!(
                observation[0], observation[2],
                "current entry during callback"
            );
        }
        let browser = self.command("Page.getNavigationHistory", json!({})).await;
        assert_eq!(browser["currentIndex"], json!(self.browser_base + index));
        let browser_urls = browser["entries"].as_array().unwrap()[self.browser_base..]
            .iter()
            .map(|entry| entry["url"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            browser_urls, urls,
            "browser history must preserve renderer commit order"
        );
    }

    fn assert_commits(&mut self, expected: &[(&str, &str)]) {
        let actual = self
            .ctx
            .sent
            .iter()
            .filter(|message| message["method"] == "Page.navigatedWithinDocument")
            .map(|message| {
                assert_eq!(message["params"]["frameId"], FRAME);
                (
                    message["params"]["url"].as_str().unwrap().to_owned(),
                    message["params"]["navigationType"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                )
            })
            .collect::<Vec<_>>();
        let expected = expected
            .iter()
            .map(|(fragment, kind)| (format!("{}{fragment}", self.base_url), (*kind).to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        self.ctx.sent.clear();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn download_interception_commits_and_settles_in_owning_window() {
    for owner in ["top", "child", "popup"] {
        let mut modes = vec![
            "empty",
            "sync",
            "reject",
            "precommit",
            "precommit-reject",
            "redirect",
        ];
        if owner == "top" {
            modes.extend([
                "reject-undefined",
                "precommit-cancel-stop",
                "precommit-cancel-navigate",
                "handler-cancel-stop",
                "handler-cancel-navigate",
            ]);
        }
        for mode in modes {
            let mut page = SameDocumentPage::new().await;
            page.allow_downloads().await;
            let script = r###"(async () => {
const ownerKind = OWNER;
const mode = MODE;
const beforeTop = location.href;
let target = window, frame;
if (ownerKind === 'child') {
  frame = document.createElement('iframe'); frame.src = 'history.html'; document.body.appendChild(frame);
  await new Promise(resolve => frame.addEventListener('load', resolve, {once:true})); target = frame.contentWindow;
} else if (ownerKind === 'popup') {
  target = open('history.html'); await new Promise(resolve => target.addEventListener('load',resolve,{once:true}));
}
await new Promise(resolve => target.setTimeout(resolve,0));
const result = await (async (target, mode) => {
  const navigation = target.navigation;
  const location = target.location;
  const document = target.document;
  const tick = () => new Promise(resolve => target.setTimeout(resolve, 0));
  const before = location.href;
  const from = navigation.currentEntry;
  const expectedURL = new URL('?download=1#one', before).href;
  const checks = {};
  const order = [];
  const states = {};
  let metadata;
  const errorDetails = [];
  const rejection = mode === 'reject-undefined' ? undefined : new Error('download interception failure');
  const precommit = mode.startsWith('precommit') || mode === 'redirect';
  const canceled = mode.includes('cancel');
  const failed = canceled || mode === 'reject' || mode === 'reject-undefined' || mode === 'precommit-reject';
  const expectedReason = () => canceled ? event.signal.reason : rejection;
  let event, transition, committed, finished, releasePrecommit, releaseHandler, nested;
  let errors = 0, successes = 0, handlerCalls = 0, precommitCalls = 0;
  const observeTransition = () => {
    const current = navigation.transition;
    checks['transition present'] = current !== null;
    if (!current) return;
    if (!transition) {
      transition = current;
      checks['transition from and to'] = current.from === from && current.to === event.destination;
      committed = current.committed.then(value => {
        states.committed = 'fulfilled';
        return value === undefined;
      }, reason => {
        states.committed = 'rejected';
        return reason === expectedReason();
      });
      finished = current.finished.then(value => {
        states.finished = 'fulfilled';
        return !failed && value === undefined;
      }, reason => {
        states.finished = 'rejected';
        return failed && reason === expectedReason();
      });
    } else checks['transition identity'] = current === transition;
  };
  for (const type of ['popstate', 'hashchange']) target.addEventListener(type, () => { if (event && !event.signal.aborted) order.push(type); });
  navigation.addEventListener('currententrychange', () => {
    if (event.signal.aborted) return;
    order.push('currententrychange');
    observeTransition();
  });
  navigation.addEventListener('navigatesuccess', () => {
    if (event.signal.aborted) return;
    order.push('navigatesuccess'); successes++;
    observeTransition();
  });
  navigation.addEventListener('navigateerror', e => {
    order.push('navigateerror'); errors++;
    checks['error identity'] = e.error === expectedReason();
    errorDetails.push({name:e.error?.name,message:e.error?.message});
    observeTransition();
  }, {once: true});
  const anchor = document.createElement(mode === 'empty' ? 'area' : 'a');
  anchor.href = expectedURL;
  anchor.download = 'example.txt';
  document.body.appendChild(anchor);
  navigation.addEventListener('navigate', e => {
    event = e;
    order.push('navigate');
    metadata = {downloadRequest:e.downloadRequest,sourceElement:e.sourceElement===anchor,navigationType:e.navigationType,
      canIntercept:e.canIntercept,cancelable:e.cancelable,userInitiated:e.userInitiated,hashChange:e.hashChange,
      sameDocument:e.destination.sameDocument,urlMatches:e.destination.url===expectedURL,
      stateKind:e.destination.getState()===null?'null':typeof e.destination.getState(),formData:e.formData,infoKind:typeof e.info};
    checks['download metadata'] = e.downloadRequest === 'example.txt' && e.sourceElement === anchor &&
      e.navigationType === 'push' && e.canIntercept && e.cancelable && !e.userInitiated &&
      !e.hashChange && !e.destination.sameDocument && e.destination.url === expectedURL &&
      e.formData === null && e.info === undefined;
    checks['download default state'] = e.destination.getState() === null;
    checks['no transition before intercept'] = navigation.transition === null;
    e.signal.addEventListener('abort', () => {
      order.push('abort');
      if (canceled || rejection !== undefined) {
        checks['abort reason'] = canceled ? e.signal.reason.name === 'AbortError' : e.signal.reason === rejection;
      }
    });
    const options = {};
    if (precommit) options.precommitHandler = controller => {
      precommitCalls++;
      order.push('precommit');
      observeTransition();
      checks['precommit URL unchanged'] = location.href === before && navigation.currentEntry === from;
      if (mode === 'precommit-reject') return Promise.reject(rejection);
      if (mode === 'redirect') controller.redirect('?redirect=1#redirect', {history:'replace',state:{download:true}});
      return new Promise(resolve => releasePrecommit = resolve);
    };
    if (mode !== 'empty') options.handler = () => {
      handlerCalls++;
      order.push('handler');
      observeTransition();
      checks['committed before handler'] = location.href === (mode === 'redirect' ? new URL('?redirect=1#redirect',before).href : expectedURL) &&
        document.URL === location.href && navigation.currentEntry.url === location.href;
      if (mode === 'reject' || mode === 'reject-undefined') return Promise.reject(rejection);
      if (mode.startsWith('handler-cancel')) return new Promise(resolve => releaseHandler = resolve);
    };
    e.intercept(options);
  }, {once:true});
  anchor.click();
  await tick();
  if (mode === 'precommit' || mode === 'redirect' || mode.startsWith('precommit-cancel')) {
    checks['precommit pending'] = precommitCalls === 1 && handlerCalls === 0 &&
      location.href === before && states.committed === undefined && states.finished === undefined;
  }
  if (canceled) {
    if (mode.endsWith('stop')) target.stop();
    else nested = navigation.navigate('#next');
  } else if (releasePrecommit) releasePrecommit();
  const observed = committed && finished ? Promise.all([committed, finished]) : Promise.resolve(null);
  const results = await Promise.race([observed, new Promise(resolve => target.setTimeout(() => resolve(null), 500))]);
  checks['both transition promises settled'] = results !== null && results.every(value => value === true);
  await tick();
  checks['terminal event'] = failed ? errors === 1 && successes === 0 : successes === 1 && errors === 0;
  checks['transition cleared'] = navigation.transition === null;
  checks['handler count'] = handlerCalls === (mode === 'empty' || mode === 'precommit-reject' || mode.startsWith('precommit-cancel') ? 0 : 1);
  checks['promise states'] = states.committed === (mode === 'precommit-reject' || mode.startsWith('precommit-cancel') ? 'rejected' : 'fulfilled') && states.finished === (failed ? 'rejected' : 'fulfilled');
  if (nested) await nested.finished;
  const finalURL = location.href;
  if (canceled) {
    releasePrecommit?.(); releaseHandler?.();
    await tick();
    checks['canceled work stays canceled'] = location.href === finalURL && navigation.transition === null;
  }
  const entries = navigation.entries().map(entry => entry.url);
  const expectedEntries = mode === 'redirect' ? [new URL('?redirect=1#redirect',before).href] :
    mode === 'precommit-reject' || mode.startsWith('precommit-cancel') ? [before] : [before,expectedURL];
  if (nested) expectedEntries.push(new URL('#next',expectedEntries.at(-1)).href);
  checks['entry history'] = JSON.stringify(entries) === JSON.stringify(expectedEntries) && navigation.currentEntry.index === entries.length - 1 &&
    location.href === entries.at(-1) && document.URL === location.href;
  checks['no legacy fragment events'] = !order.includes('popstate') && !order.includes('hashchange');
  if (mode === 'redirect') checks['redirect state'] = event.destination.getState()?.download === true && navigation.currentEntry.getState() === undefined;
  return {mode,metadata,errorDetails,checks,order,states,entries,index:navigation.currentEntry.index,href:location.href};
})(target, mode);
if (ownerKind !== 'top') {
  result.checks['parent unchanged'] = location.href === beforeTop && navigation.transition === null;
  if (frame) frame.remove(); else target.close();
}
return result;
})()"###
                .replace("OWNER", &json!(owner).to_string())
                .replace("MODE", &json!(mode).to_string());
            let result = page.evaluate(&script).await;
            assert!(
                result["checks"]
                    .as_object()
                    .unwrap()
                    .values()
                    .all(|value| value == true),
                "{owner}/{mode}: {result}"
            );
            assert_eq!(
                page.download_requests
                    .load(std::sync::atomic::Ordering::SeqCst),
                0,
                "intercepted {owner}/{mode} must not fetch a download"
            );
            if owner == "top" {
                let (urls, commits): (&[&str], &[(&str, &str)]) = match mode {
                    "precommit-reject" | "precommit-cancel-stop" => (&[""], &[]),
                    "precommit-cancel-navigate" => (&["", "#next"], &[("#next", "fragment")]),
                    "handler-cancel-navigate" => (
                        &["", "?download=1#one", "?download=1#next"],
                        &[
                            ("?download=1#one", "other"),
                            ("?download=1#next", "fragment"),
                        ],
                    ),
                    "redirect" => (
                        &["?redirect=1#redirect"],
                        &[("?redirect=1#redirect", "other")],
                    ),
                    _ => (&["", "?download=1#one"], &[("?download=1#one", "other")]),
                };
                page.assert_history(urls, urls.len() - 1).await;
                page.assert_commits(commits);
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_transition_committed_resolves_without_a_history_entry() {
    for operation in ["navigate", "reload", "back"] {
        for precommit in [false, true] {
            let mut page = SameDocumentPage::new().await;
            page.run(
                "history.pushState(null, '', '#current'); void 0",
                "#current",
            )
            .await;
            let result = page.evaluate(&format!(r#"(async () => {{
                const operation = {operation};
                const precommit = {precommit};
                let transition;
                navigation.addEventListener('navigate', event => event.intercept({{
                    ...(precommit ? {{precommitHandler() {{transition = navigation.transition;}}}} : {{}}),
                    handler() {{transition = navigation.transition;}}
                }}), {{once: true}});
                const result = operation === 'navigate' ? navigation.navigate('#next') : navigation[operation]();
                const [committed, finished] = await Promise.all([result.committed, result.finished]);
                return committed === navigation.currentEntry && finished === committed &&
                    await transition.committed === undefined && await transition.finished === undefined;
            }})()"#, operation=json!(operation))).await;
            assert_eq!(result, true, "{operation}, precommit={precommit}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn active_precommit_cancellation_rejects_owned_transition() {
    for operation in ["back", "navigate", "reload"] {
        for action in ["stop", "navigate", "intercept"] {
            let mut page = SameDocumentPage::new().await;
            page.run(
                "history.pushState(null, '', '#current'); void 0",
                "#current",
            )
            .await;
            page.ctx.sent.clear();
            let script = r#"(async () => {
  const operation = OPERATION;
  const action = ACTION;
  const from = navigation.currentEntry;
  const checks = [];
  const states = {};
  const reasons = [];
  const order = [];
  let transition, signal, nested, nestedTransition, releaseNested;
  let handlerRan = false;
  let nestedFinished = false;
  const rejected = (name, promise) => promise.then(
    () => { states[name] = 'fulfilled'; },
    reason => { states[name] = 'rejected'; reasons.push(reason); }
  );
  navigation.addEventListener('navigateerror', event => {
    order.push('navigateerror');
    checks.push(event.error === signal.reason);
  }, {once: true});
  navigation.addEventListener('navigate', event => {
    signal = event.signal;
    signal.addEventListener('abort', () => {
      order.push('abort');
      checks.push(signal.reason instanceof DOMException && signal.reason.name === 'AbortError');
    }, {once: true});
    event.intercept({
      precommitHandler() {
        order.push('precommit');
        transition = navigation.transition;
        checks.push(transition !== null && transition.from === from && transition.to === event.destination);
        checks.push(transition?.navigationType === (operation === 'back' ? 'traverse' : operation === 'navigate' ? 'push' : 'reload'));
        rejected('transition committed', transition.committed);
        rejected('transition finished', transition.finished);
        if (action === 'stop') {
          window.stop();
        } else {
          if (action === 'intercept') {
            navigation.addEventListener('navigate', event => event.intercept({handler() {
              nestedTransition = navigation.transition;
              return new Promise(resolve => releaseNested = resolve);
            }}), {once: true});
          }
          nested = navigation.navigate('#nested');
          nested.finished.then(() => nestedFinished = true);
        }
        checks.push(signal.aborted);
      },
      handler() { handlerRan = true; }
    });
  }, {once: true});
  const result = operation === 'navigate' ? navigation.navigate('#outer') : navigation[operation]();
  await Promise.all([
    rejected('method committed', result.committed),
    rejected('method finished', result.finished)
  ]);
  await new Promise(resolve => setTimeout(resolve, 0));
  checks.push(!handlerRan, reasons.length === 4 && reasons.every(reason => reason === signal.reason));
  if (action === 'intercept') {
    checks.push(navigation.transition === nestedTransition && nestedTransition !== null && nestedTransition !== transition);
    checks.push(!nestedFinished);
    releaseNested();
  } else {
    checks.push(navigation.transition === null);
  }
  if (nested) await nested.finished;
  checks.push(navigation.transition === null);
  return {states, checks, order};
})()
"#
                .replace("OPERATION", &json!(operation).to_string())
                .replace("ACTION", &json!(action).to_string());
            let result = page.evaluate(&script).await;
            assert_eq!(
                result["states"],
                json!({
                    "transition committed": "rejected",
                    "transition finished": "rejected",
                    "method committed": "rejected",
                    "method finished": "rejected",
                }),
                "{operation}/{action}: {result}"
            );
            assert!(
                result["checks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|check| check == true),
                "{operation}/{action}: {result}"
            );
            assert_eq!(
                result["order"],
                json!(["precommit", "abort", "navigateerror"]),
                "{operation}/{action}"
            );
            if action == "stop" {
                page.assert_history(&["", "#current"], 1).await;
                page.assert_commits(&[]);
            } else {
                page.assert_history(&["", "#current", "#nested"], 2).await;
                page.assert_commits(&[(
                    "#nested",
                    if action == "intercept" {
                        "other"
                    } else {
                        "fragment"
                    },
                )]);
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn intercepted_traversal_transitions_track_commit_and_completion() {
    for precommit in [false, true] {
        for mode in ["empty", "sync", "async", "reject", "undefined"] {
            let mut page = SameDocumentPage::new().await;
            page.run(
                "history.pushState(null, '', '#back'); history.pushState(null, '', '#current'); void 0",
                "#current",
            ).await;
            page.ctx.sent.clear();
            let script = r#"
                (async () => {
                    const mode = MODE;
                    const precommit = PRECOMMIT;
                    const order = [];
                    const checks = [];
                    const from = navigation.currentEntry;
                    const error = mode === 'undefined' ? undefined : new Error('traversal failure');
                    const fails = mode === 'reject' || mode === 'undefined';
                    let transition;
                    let event;
                    let transitionCommitted;
                    let transitionFinished;
                    const capture = phase => {
                        order.push(phase);
                        const current = navigation.transition;
                        checks.push(current !== null && current.from === from &&
                            current.navigationType === 'traverse' && current.to === event.destination);
                        if (!transition) {
                            transition = current;
                            transitionCommitted = transition?.committed.then(value => value === undefined);
                            transitionFinished = transition?.finished.then(
                                value => { order.push('transition finished'); return !fails && value === undefined; },
                                reason => { order.push('transition finished'); return fails && reason === error; }
                            );
                        }
                        checks.push(current === transition);
                    };
                    navigation.addEventListener('navigate', e => {
                        event = e;
                        order.push('navigate');
                        checks.push(navigation.transition === null);
                        const options = {};
                        if (precommit) options.precommitHandler = () => {
                            capture('precommit');
                            checks.push(location.hash === '#current');
                            return new Promise(resolve => setTimeout(resolve, 0));
                        };
                        if (mode !== 'empty') options.handler = () => {
                            capture('handler');
                            checks.push(location.hash === '#back');
                            if (fails) return Promise.reject(error);
                            if (mode === 'async') return new Promise(resolve => setTimeout(resolve, 0));
                        };
                        e.intercept(options);
                        e.signal.addEventListener('abort', () => checks.push(fails && e.signal.reason === error));
                    }, {once: true});
                    navigation.addEventListener('currententrychange', () => capture('currententrychange'), {once: true});
                    navigation.addEventListener('navigatesuccess', () => capture('success'), {once: true});
                    navigation.addEventListener('navigateerror', e => {
                        capture('error');
                        checks.push(fails && e.error === error);
                    }, {once: true});
                    const result = navigation.back();
                    const committed = result.committed.then(entry => {
                        capture('committed');
                        return entry === navigation.currentEntry;
                    });
                    const finished = result.finished.then(
                        entry => { order.push('finished'); return !fails && entry === navigation.currentEntry && navigation.transition === null; },
                        reason => { order.push('finished'); return fails && reason === error && navigation.transition === null; }
                    );
                    checks.push(await committed, await finished);
                    checks.push(await transitionCommitted, await transitionFinished);
                    return {order, checks};
                })()
            "#.replace("MODE", &json!(mode).to_string())
                .replace("PRECOMMIT", if precommit { "true" } else { "false" });
            let result = page.evaluate(&script).await;
            let mut expected = vec!["navigate"];
            if precommit {
                expected.push("precommit");
            }
            expected.push("currententrychange");
            if mode != "empty" {
                expected.push("handler");
            }
            expected.extend([
                "committed",
                if matches!(mode, "reject" | "undefined") {
                    "error"
                } else {
                    "success"
                },
                "finished",
                "transition finished",
            ]);
            assert_eq!(
                result["order"],
                json!(expected),
                "{mode}, precommit={precommit}"
            );
            assert!(
                result["checks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|value| value == true),
                "{mode}, precommit={precommit}: {result}"
            );
            page.assert_history(&["", "#back", "#current"], 1).await;
            page.assert_commits(&[("#back", "other")]);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn intercepted_traversal_transitions_settle_when_canceled() {
    for phase in ["precommit", "handler"] {
        for action in ["stop", "navigate"] {
            let mut page = SameDocumentPage::new().await;
            let script = r#"(async () => {
  const phase = PHASE;
  const action = ACTION;
  history.pushState(null, '', '#back');
  history.pushState(null, '', '#current');
  await new Promise(resolve=>setTimeout(resolve,0));
  const from = navigation.currentEntry;
  let ready;
  const started = new Promise(resolve => ready = resolve);
  let transition, signal, release;
  const observed = {committed: 'pending', finished: 'pending', transitionCommitted: 'pending', transitionFinished: 'pending'};
  navigation.addEventListener('navigate', event => {
    signal = event.signal;
    const handler = () => {
      transition = navigation.transition;
      observed.transitionPresent = transition !== null;
      observed.from = transition?.from === from;
      transition?.committed?.then(() => observed.transitionCommitted = 'resolved', reason => observed.transitionCommitted = reason.name);
      transition?.finished.then(() => observed.transitionFinished = 'resolved', reason => observed.transitionFinished = reason.name);
      ready();
      return new Promise(resolve => release = resolve);
    };
    event.intercept(phase === 'precommit' ? {precommitHandler: handler} : {handler});
  }, {once: true});
  const result = navigation.back();
  result.committed.then(() => observed.committed = 'resolved', reason => observed.committed = reason.name);
  result.finished.then(() => observed.finished = 'resolved', reason => observed.finished = reason.name);
  await started;
  if (action === 'stop') window.stop();
  else await navigation.navigate('#nested').finished;
  await result.finished.catch(() => {});
  release();
  await new Promise(resolve => setTimeout(resolve, 0));
  observed.currentTransition = navigation.transition;
  observed.hash = location.hash;
  observed.aborted = signal.aborted;
  return observed;
})()
"#
                .replace("PHASE", &json!(phase).to_string())
                .replace("ACTION", &json!(action).to_string());
            let result = page.evaluate(&script).await;
            let committed = if phase == "precommit" {
                "AbortError"
            } else {
                "resolved"
            };
            let hash = if action == "navigate" {
                "#nested"
            } else if phase == "precommit" {
                "#current"
            } else {
                "#back"
            };
            assert_eq!(
                result,
                json!({
                    "committed": committed,
                    "finished": "AbortError",
                    "transitionCommitted": committed,
                    "transitionFinished": "AbortError",
                    "transitionPresent": true,
                    "from": true,
                    "currentTransition": null,
                    "hash": hash,
                    "aborted": true,
                }),
                "phase={phase}, action={action}"
            );
            if action == "stop" {
                page.assert_history(
                    &["", "#back", "#current"],
                    if phase == "precommit" { 2 } else { 1 },
                )
                .await;
            } else if phase == "precommit" {
                page.assert_history(&["", "#back", "#current", "#nested"], 3)
                    .await;
            } else {
                page.assert_history(&["", "#back", "#nested"], 2).await;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn intercepted_traversal_transitions_reject_before_commit() {
    for undefined in [false, true] {
        let mut page = SameDocumentPage::new().await;
        page.run(
            "history.pushState(null, '', '#current'); void 0",
            "#current",
        )
        .await;
        page.ctx.sent.clear();
        let script = r#"(async () => {
  const expected = UNDEFINED ? undefined : new Error('precommit failure');
  const from = navigation.currentEntry;
  const checks = [];
  let transition;
  let handlerRan = false;
  navigation.addEventListener('navigate', event => {
    event.signal.addEventListener('abort', () => checks.push(event.signal.reason === expected));
    event.intercept({
      precommitHandler() {
        transition = navigation.transition;
        return new Promise((_, reject) => setTimeout(() => reject(expected), 0));
      },
      handler() { handlerRan = true; }
    });
  }, {once: true});
  const result = navigation.back();
  const rejectedWithExpected = promise => promise.then(() => false, reason => reason === expected);
  checks.push(...await Promise.all([result.committed, result.finished].map(rejectedWithExpected)));
  checks.push(...await Promise.all([transition.committed, transition.finished].map(rejectedWithExpected)));
  checks.push(!handlerRan, navigation.currentEntry === from, navigation.transition === null);
  return checks;
})()
"#.replace("UNDEFINED", if undefined { "true" } else { "false" });
        let result = page.evaluate(&script).await;
        assert_eq!(
            result,
            json!([true, true, true, true, true, true, true, true]),
            "undefined={undefined}"
        );
        page.assert_history(&["", "#current"], 1).await;
        page.assert_commits(&[]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn intercepted_traversal_transitions_preserve_navigation_started_during_completion() {
    for trigger in ["success", "error", "abort"] {
        let mut page = SameDocumentPage::new().await;
        page.run(
            "history.pushState(null, '', '#back'); history.pushState(null, '', '#current'); void 0",
            "#current",
        )
        .await;
        page.ctx.sent.clear();
        let script = r#"(async () => {
  const trigger = TRIGGER;
  const error = new Error('expected');
  const observed = {};
  let transition, nestedTransition, nested, release;
  navigation.addEventListener('currententrychange', () => {
    transition = navigation.transition;
    transition?.finished.then(() => observed.oldFinished = 'resolved', reason => observed.oldFinished = reason === error ? 'expected' : reason.name);
  }, {once: true});
  const startNested = () => {
    navigation.addEventListener('navigate', e => e.intercept({handler() {
      nestedTransition = navigation.transition;
      return new Promise(resolve => release = resolve);
    }}), {once: true});
    nested = navigation.navigate('#nested');
    nested.finished.then(() => observed.nestedFinished = true, reason => observed.nestedError = reason.name);
  };
  navigation.addEventListener('navigate', e => {
    if (trigger === 'abort') e.signal.addEventListener('abort', startNested, {once: true});
    e.intercept({handler() {
      if (trigger !== 'success') return Promise.reject(error);
    }});
  }, {once: true});
  if (trigger !== 'abort') navigation.addEventListener(trigger === 'success' ? 'navigatesuccess' : 'navigateerror', startNested, {once: true});
  const result = navigation.back();
  await result.finished.catch(() => {});
  await new Promise(resolve => setTimeout(resolve, 0));
  observed.preservedNewTransition = navigation.transition === nestedTransition && nestedTransition !== null;
  observed.distinct = nestedTransition !== transition;
  observed.newPending = !observed.nestedFinished;
  release();
  await nested.finished.catch(() => {});
  await new Promise(resolve => setTimeout(resolve, 0));
  observed.cleared = navigation.transition === null;
  return observed;
})()
"#.replace("TRIGGER", &json!(trigger).to_string());
        let result = page.evaluate(&script).await;
        assert_eq!(
            result,
            json!({
                "oldFinished": if trigger == "success" { "resolved" } else { "expected" },
                "preservedNewTransition": true,
                "distinct": true,
                "newPending": true,
                "nestedFinished": true,
                "cleared": true,
            }),
            "trigger={trigger}"
        );
        page.assert_history(&["", "#back", "#nested"], 2).await;
        page.assert_commits(&[("#back", "other"), ("#nested", "other")]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn same_document_commits_keep_navigation_api_and_browser_history_in_sync() {
    let mut page = SameDocumentPage::new().await;
    page.run("navigation.navigate('#one').finished", "#one")
        .await;
    page.assert_history(&["", "#one"], 1).await;
    page.assert_commits(&[("#one", "fragment")]);

    page.run(
        "navigation.navigate('#two', {history: 'replace'}).finished",
        "#two",
    )
    .await;
    page.assert_history(&["", "#two"], 1).await;
    page.assert_commits(&[("#two", "fragment")]);

    page.run("location.hash = 'three'; void 0", "#three").await;
    page.assert_history(&["", "#two", "#three"], 2).await;
    page.assert_commits(&[("#three", "fragment")]);

    page.run("navigation.back().finished", "#two").await;
    page.assert_history(&["", "#two", "#three"], 1).await;
    page.assert_commits(&[("#two", "fragment")]);

    page.run("history.forward()", "#three").await;
    page.assert_history(&["", "#two", "#three"], 2).await;
    page.assert_commits(&[("#three", "fragment")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn same_document_commits_precede_reentrant_author_callbacks() {
    for (outer, first_kind, listener, nested, second_kind) in [
        (
            "navigation.navigate('#one').finished.catch(() => {})",
            "fragment",
            "navigation.addEventListener('currententrychange',",
            "history.pushState(null, '', '#two')",
            "historyApi",
        ),
        (
            "navigation.navigate('#one').finished.catch(() => {})",
            "fragment",
            "navigation.addEventListener('currententrychange',",
            "location.hash = 'two'",
            "fragment",
        ),
        (
            "history.pushState(null, '', '#one')",
            "historyApi",
            "navigation.addEventListener('currententrychange',",
            "navigation.navigate('#two').finished.catch(() => {})",
            "fragment",
        ),
        (
            "history.pushState(null, '', '#one')",
            "historyApi",
            "navigation.addEventListener('currententrychange',",
            "location.hash = 'two'",
            "fragment",
        ),
        (
            "location.hash = 'one'",
            "fragment",
            "navigation.addEventListener('currententrychange',",
            "history.pushState(null, '', '#two')",
            "historyApi",
        ),
        (
            "location.hash = 'one'",
            "fragment",
            "addEventListener('popstate',",
            "history.pushState(null, '', '#two')",
            "historyApi",
        ),
        (
            "navigation.navigate('#one').finished.catch(() => {})",
            "fragment",
            "addEventListener('popstate',",
            "history.pushState(null, '', '#two')",
            "historyApi",
        ),
    ] {
        let mut page = SameDocumentPage::new().await;
        page.run(
            &format!("{listener} () => {{ {nested}; }}, {{once: true}}); {outer}; void 0"),
            "#two",
        )
        .await;
        page.assert_history(&["", "#one", "#two"], 2).await;
        page.assert_commits(&[("#one", first_kind), ("#two", second_kind)]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn same_document_commits_publish_traversal_before_nested_push_state() {
    for listener in [
        "navigation.addEventListener('currententrychange',",
        "addEventListener('popstate',",
    ] {
        let mut page = SameDocumentPage::new().await;
        page.run(
            "history.pushState(null, '', '#one'); history.pushState(null, '', '#two');",
            "#two",
        )
        .await;
        page.assert_commits(&[("#one", "historyApi"), ("#two", "historyApi")]);
        page.run(&format!("{listener} () => history.pushState(null, '', '#nested'), {{once: true}}); history.back();"), "#nested").await;
        page.assert_history(&["", "#one", "#nested"], 2).await;
        page.assert_commits(&[("#one", "fragment"), ("#nested", "historyApi")]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn same_document_commits_distinguish_interception_and_cancellation() {
    for navigate in [
        "navigation.navigate('#one').finished",
        "location.hash = 'one'; void 0",
    ] {
        let mut page = SameDocumentPage::new().await;
        page.run(&format!("navigation.addEventListener('navigate', e => e.intercept({{handler: () => Promise.resolve()}}), {{once: true}}); {navigate}"), "#one").await;
        page.assert_history(&["", "#one"], 1).await;
        page.assert_commits(&[("#one", "other")]);
    }

    for handler in [
        "handler: () => Promise.resolve()",
        "precommitHandler: () => Promise.resolve()",
    ] {
        let mut page = SameDocumentPage::new().await;
        page.run(
            "history.pushState(null, '', '#one'); history.pushState(null, '', '#two');",
            "#two",
        )
        .await;
        page.assert_commits(&[("#one", "historyApi"), ("#two", "historyApi")]);
        page.run(&format!("navigation.addEventListener('navigate', e => e.intercept({{{handler}}}), {{once: true}}); navigation.back().finished"), "#one").await;
        page.assert_history(&["", "#one", "#two"], 1).await;
        page.assert_commits(&[("#one", "other")]);
    }

    let mut page = SameDocumentPage::new().await;
    page.evaluate("navigation.addEventListener('navigate', e => e.preventDefault(), {once: true}); navigation.navigate('#one').finished.catch(e => e.name)").await;
    page.assert_history(&[""], 0).await;
    page.assert_commits(&[]);
}

#[tokio::test(flavor = "multi_thread")]
async fn intercepted_navigation_commits_do_not_dispatch_fragment_events() {
    for history in ["push", "replace"] {
        for suffix in ["#one", "?route#one"] {
            for mode in ["empty", "handler", "precommit", "reject"] {
                let mut page = SameDocumentPage::new().await;
                page.run("history.replaceState({classic: 1}, '', '#seed')", "#seed")
                    .await;
                page.assert_commits(&[("#seed", "historyApi")]);
                page.evaluate(
                    r#"
                    globalThis.legacyEvents = [];
                    globalThis.handlerSnapshots = [];
                    for (const type of ['popstate', 'hashchange']) {
                        addEventListener(type, () => legacyEvents.push(type));
                    }
                    globalThis.expectedError = new Error('handler failed');
                    globalThis.handler = () => {
                        handlerSnapshots.push(legacyEvents.slice());
                    };
                    void 0;
                "#,
                )
                .await;
                let options = match mode {
                    "empty" => "{}",
                    "handler" => "{handler}",
                    "precommit" => {
                        "{precommitHandler: () => new Promise(resolve => setTimeout(resolve, 0)), handler}"
                    }
                    "reject" => "{handler() { handler(); throw expectedError; }}",
                    _ => unreachable!(),
                };
                page.run(&format!(r#"
                    navigation.addEventListener('navigate', event => event.intercept({options}), {{once: true}});
                    navigation.navigate('{suffix}', {{history: '{history}', state: {{api: 1}}}}).finished.then(
                        () => globalThis.outcome = 'fulfilled',
                        error => {{
                            if (error !== expectedError) throw error;
                            globalThis.outcome = 'rejected';
                        }}
                    );
                "#), suffix).await;
                if history == "push" {
                    page.assert_history(&["#seed", suffix], 1).await;
                } else {
                    page.assert_history(&[suffix], 0).await;
                }
                page.assert_commits(&[(suffix, "other")]);
                let observed = page
                    .evaluate(
                        r#"({
                    legacyEvents, handlerSnapshots, outcome,
                    classicState: history.state,
                    navigationState: navigation.currentEntry.getState(),
                })"#,
                    )
                    .await;
                assert_eq!(
                    observed,
                    json!({
                        "legacyEvents": [],
                        "handlerSnapshots": if mode == "empty" { json!([]) } else { json!([[]]) },
                        "outcome": if mode == "reject" { "rejected" } else { "fulfilled" },
                        "classicState": null,
                        "navigationState": {"api": 1},
                    }),
                    "{history} {suffix} {mode}"
                );
            }
        }
    }

    let mut page = SameDocumentPage::new().await;
    page.evaluate("globalThis.legacyEvents = []; for (const type of ['popstate', 'hashchange']) addEventListener(type, () => legacyEvents.push(type));").await;
    page.run("navigation.navigate('#one').finished", "#one")
        .await;
    assert_eq!(
        page.evaluate("legacyEvents").await,
        json!(["popstate", "hashchange"])
    );
}
