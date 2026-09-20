use super::*;

mod child_history_traversal;
mod popup_network_navigation;
mod response_gate;

use response_gate::ResponseGate;

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
        Self::with_routes(axum::Router::new()).await
    }

    async fn with_routes(routes: axum::Router) -> Self {
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
            axum::serve(listener, app.merge(routes)).await.unwrap();
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
async fn precommit_rejections_preserve_promise_and_event_order() {
    for (owner, operation, mode) in [
        ("top", "navigate", "reject"),
        ("top", "navigate", "stop"),
        ("top", "navigate", "navigate"),
        ("top", "reload", "reject"),
        ("top", "reload", "stop"),
        ("top", "reload", "navigate"),
        ("top", "location", "reject"),
        ("top", "location", "stop"),
        ("top", "location", "navigate"),
        ("top", "download", "reject"),
        ("top", "download", "stop"),
        ("top", "download", "navigate"),
        ("top", "back", "reject"),
        ("top", "back", "stop"),
        ("top", "back", "navigate"),
        ("top", "anchor", "reject"),
        ("top", "anchor", "stop"),
        ("top", "anchor", "navigate"),
        ("top", "form", "reject"),
        ("top", "form", "stop"),
        ("top", "form", "navigate"),
        ("child", "navigate", "reject"),
        ("child", "navigate", "stop"),
        ("child", "navigate", "navigate"),
        ("child", "reload", "reject"),
        ("child", "reload", "stop"),
        ("child", "reload", "navigate"),
        ("child", "download", "reject"),
        ("child", "download", "stop"),
        ("child", "download", "navigate"),
        ("popup", "navigate", "reject"),
        ("popup", "navigate", "stop"),
        ("popup", "navigate", "navigate"),
        ("popup", "reload", "reject"),
        ("popup", "reload", "stop"),
        ("popup", "reload", "navigate"),
        ("popup", "download", "reject"),
        ("popup", "download", "stop"),
        ("popup", "download", "navigate"),
        ("top", "navigate", "primitive"),
        ("top", "reload", "primitive"),
        ("top", "back", "primitive"),
        ("top", "location", "primitive"),
        ("top", "download", "primitive"),
    ] {
        let mut page = SameDocumentPage::new().await;
        page.allow_downloads().await;
        let script = r###"(async () => {
  const ownerKind=OWNER, operation=OPERATION, mode=MODE;
  const parentURL=location.href;
  let target=window, frame;
  if(ownerKind==='child') {
    frame=document.createElement('iframe');frame.src='history.html';document.body.appendChild(frame);
    await new Promise(resolve=>frame.addEventListener('load',resolve,{once:true}));target=frame.contentWindow;
  } else if(ownerKind==='popup') {
    target=open('history.html');await new Promise(resolve=>target.addEventListener('load',resolve,{once:true}));
  }
  const nav=target.navigation, loc=target.location, doc=target.document;
  const tick=()=>new Promise(resolve=>setTimeout(resolve,0));
  if(operation==='back') target.history.pushState(null,'','#current');
  await tick();
  const before=loc.href, from=nav.currentEntry;
  const order=[], checks={}, reasons=[], states={}, promises=[];
  const error=mode==='primitive'?'precommit failure':new Error('precommit failure');
  let event, transition, method, nested, rejectGate, releaseGate, handlerCalls=0;
  const gate=new Promise((resolve,reject)=>{releaseGate=resolve;rejectGate=reject;});
  const observe=(name,promise)=>promises.push(promise.then(
    ()=>{states[name]='fulfilled';order.push(name+' fulfilled');},
    reason=>{states[name]='rejected';reasons.push(reason);order.push(name+' rejected');}
  ));
  nav.addEventListener('navigateerror',e=>{
    order.push('navigateerror');reasons.push(e.error);
    checks['transition during error']=nav.transition===transition;
    queueMicrotask(()=>order.push('error microtask'));
  },{once:true});
  nav.addEventListener('navigate',e=>{
    event=e;
    e.signal.addEventListener('abort',()=>{
      order.push('abort');reasons.push(e.signal.reason);
      checks['transition during abort']=nav.transition===transition;
      queueMicrotask(()=>order.push('abort microtask'));
    },{once:true});
    e.intercept({precommitHandler(){
      order.push('precommit');transition=nav.transition;
      observe('transition committed',transition.committed);observe('transition finished',transition.finished);
      return gate;
    },handler(){handlerCalls++;}});
  },{once:true});
  if(operation==='navigate')method=nav.navigate('?requested=1');
  else if(operation==='location')loc.assign('?requested=1');
  else if(operation==='download'||operation==='anchor'){
    const a=doc.createElement('a');a.href=operation==='download'?'?download=1':'?requested=1';
    if(operation==='download')a.download='test.txt';doc.body.appendChild(a);a.click();
  } else if(operation==='form'){
    const form=doc.createElement('form');form.action='?requested=1';form.method='post';doc.body.appendChild(form);form.requestSubmit();
  } else method=nav[operation]();
  if(method){observe('method committed',method.committed);observe('method finished',method.finished);}
  for(let i=0;i<60&&!transition;i++)await tick();
  checks.started=transition!=null&&nav.currentEntry===from;
  if(mode==='stop')target.stop();
  else if(mode==='navigate')nested=nav.navigate('#next');
  else rejectGate(error);
  checks.settled=await Promise.race([Promise.all(promises).then(()=>true),new Promise(resolve=>setTimeout(()=>resolve(false),700))]);
  if(nested)await nested.finished;
  releaseGate();await tick();
  const methodOrder=method?['method committed rejected','method finished rejected']:[];
  checks.order=JSON.stringify(order)===JSON.stringify(['precommit','abort','navigateerror','abort microtask',...methodOrder,'error microtask','transition committed rejected','transition finished rejected']);
  checks['promise states']=Object.keys(states).length===(method?4:2)&&Object.values(states).every(v=>v==='rejected');
  const canceled=mode==='stop'||mode==='navigate', expected=canceled?event.signal.reason:error;
  checks['error identity']=reasons.length===(method?6:4)&&reasons.every(reason=>reason===expected);
  checks['signal reason']=!canceled||event.signal.reason.name==='AbortError';
  checks['handler suppressed']=handlerCalls===0;
  checks['transition cleared']=nav.transition===null;
  const expectedURL=mode==='navigate'?new URL('#next',before).href:before;
  checks['final URL']=loc.href===expectedURL&&doc.URL===loc.href&&nav.currentEntry.url===loc.href;
  const result={ownerKind,operation,mode,checks,order,states,href:loc.href,entries:nav.entries().map(e=>e.url)};
  if(ownerKind!=='top') {
    checks['parent unchanged']=location.href===parentURL&&navigation.transition===null;
    if(frame)frame.remove();else target.close();
  }
  return result;
})()
"###
            .replace("OWNER", &json!(owner).to_string())
            .replace("OPERATION", &json!(operation).to_string())
            .replace("MODE", &json!(mode).to_string());
        let result = page.evaluate(&script).await;
        assert!(
            result["checks"]
                .as_object()
                .unwrap()
                .values()
                .all(|value| value == true),
            "{owner}/{operation}/{mode}: {result}"
        );
        assert_eq!(
            page.download_requests
                .load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a rejected intercepted download must not be fetched"
        );
        if owner == "top" {
            match (operation, mode) {
                ("back", "navigate") => page.assert_history(&["", "#current", "#next"], 2).await,
                ("back", _) => page.assert_history(&["", "#current"], 1).await,
                (_, "navigate") => page.assert_history(&["", "#next"], 1).await,
                _ => page.assert_history(&[""], 0).await,
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn window_stop_preserves_cancellation_during_reentrant_callbacks() {
    for action in [
        "window.stop()",
        "child.stop()",
        "sibling.frameElement.remove()",
        "sibling.document.open(); sibling.document.write('<!doctype html><p>replacement</p>'); sibling.document.close()",
    ] {
        let mut page = SameDocumentPage::new().await;
        let script = r###"(async()=>{
const tick=()=>new Promise(r=>setTimeout(r,0));
async function frame(w,name,shadow=false){const f=w.document.createElement('iframe');const loaded=new Promise(r=>f.onload=r);f.src='history.html?'+name;let root=w.document.body;if(shadow){const h=w.document.createElement('div');root.appendChild(h);root=h.attachShadow({mode:'closed'});}root.appendChild(f);await loaded;return f.contentWindow;}
const child=await frame(window,'child'),sibling=await frame(window,'sibling'),grand=await frame(child,'grand'),shadow=await frame(window,'shadow',true);
const popup=open('history.html?popup');await new Promise(r=>popup.onload=r);await tick();
const worlds={top:window,child,sibling,grand,shadow,popup};
const events=[],signals={},transitions={},releases={},results={},states={},before={};
for(const [name,w] of Object.entries(worlds)){
 before[name]={href:w.location.href,length:w.navigation.entries().length,entry:w.navigation.currentEntry};
 w.navigation.addEventListener('navigate',e=>{signals[name]=e.signal;e.signal.addEventListener('abort',()=>events.push('abort:'+name));e.intercept({precommitHandler(){transitions[name]=w.navigation.transition;transitions[name].committed.catch(()=>{});transitions[name].finished.catch(()=>{});return new Promise(r=>releases[name]=r);}});},{once:true});
 w.navigation.addEventListener('navigateerror',()=>events.push('error:'+name),{once:true});
 results[name]=w.navigation.navigate('?requested='+name);
 states[name]='pending';results[name].committed.then(()=>states[name]='fulfilled',e=>states[name]=e.name);results[name].finished.catch(()=>{});
}
signals.grand.addEventListener('abort',()=>{ ACTION; },{once:true});
await tick();
const snap=()=>({states:{...states},aborted:Object.fromEntries(Object.entries(signals).map(([n,s])=>[n,s.aborted])),events:[...events]});
window.stop();
const sync=snap();await Promise.resolve();const micro=snap();await tick();await tick();const stopped=snap();
const retained={};
Object.values(releases).forEach(r=>r());await Promise.allSettled(Object.values(results).map(r=>r.finished));await tick();
const final=snap();popup.close();return{sync,micro,stopped,retained,final};
})()"###.replace("ACTION", action);
        let result = page.evaluate(&script).await;
        assert_eq!(
            result["stopped"]["aborted"],
            json!({
                "top": true, "child": true, "sibling": true,
                "grand": true, "shadow": true, "popup": false,
            }),
            "{action}: {result}"
        );
        assert_eq!(
            result["final"]["states"],
            json!({
                "top": "AbortError", "child": "AbortError", "sibling": "AbortError",
                "grand": "AbortError", "shadow": "AbortError", "popup": "fulfilled",
            }),
            "{action}: {result}"
        );
        let expected_events = if action == "sibling.frameElement.remove()" {
            vec![
                "abort:grand",
                "abort:sibling",
                "error:sibling",
                "error:grand",
                "abort:child",
                "error:child",
                "abort:shadow",
                "error:shadow",
                "abort:top",
                "error:top",
            ]
        } else {
            vec![
                "abort:grand",
                "error:grand",
                "abort:child",
                "error:child",
                "abort:sibling",
                "error:sibling",
                "abort:shadow",
                "error:shadow",
                "abort:top",
                "error:top",
            ]
        };
        assert_eq!(
            result["stopped"]["events"],
            json!(expected_events),
            "{action}: {result}"
        );
        assert_eq!(
            result["final"]["events"], result["stopped"]["events"],
            "cancellation must not repeat after releasing the precommit gates"
        );
        page.assert_history(&[""], 0).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn window_stop_validates_receivers_and_ignores_discarded_windows() {
    for (action, detached) in [
        ("window.stop.call({})", false),
        ("window.stop.call(Object.create(window))", false),
        ("window.stop.call(new Proxy(child, {}))", false),
        (
            "let p = Proxy.revocable(child, {}); p.revoke(); window.stop.call(p.proxy)",
            false,
        ),
        ("f.remove(); oldStop.call(child)", true),
        ("f.remove(); window.stop.call(child)", true),
    ] {
        let mut page = SameDocumentPage::new().await;
        let script = r###"(async()=>{
const tick=()=>new Promise(r=>setTimeout(r,0));
const f=document.createElement('iframe');f.src='history.html?old';const loaded=new Promise(r=>f.onload=r);document.body.appendChild(f);await loaded;await tick();
const child=f.contentWindow, oldStop=child.stop, results={}, states={}, signals={}, releases={};
for(const [name,w] of [['top',window],['child',child]]){
 w.navigation.addEventListener('navigate',e=>{signals[name]=e.signal;e.intercept({precommitHandler(){return new Promise(r=>releases[name]=r);}});},{once:true});
 results[name]=w.navigation.navigate('?requested='+name);results[name].committed.then(()=>states[name]='fulfilled',e=>states[name]=e.name);results[name].finished.catch(()=>{});
}
let outcome;
try{ACTION;outcome='ok';}catch(e){outcome=e.name;}
await tick();const stopped={outcome,states:{...states},topAborted:signals.top.aborted,childAborted:signals.child.aborted};
releases.top();releases.child();await Promise.allSettled(Object.values(results).map(r=>r.finished));return stopped;
})()"###.replace("ACTION", action);
        let result = page.evaluate(&script).await;
        assert_eq!(
            result,
            json!({
                "outcome": if detached { "ok" } else { "TypeError" },
                "states": if detached { json!({"child": "AbortError"}) } else { json!({}) },
                "topAborted": false,
                "childAborted": detached,
            }),
            "{action}: {result}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn window_stop_cancels_receiver_and_descendant_precommit_navigations() {
    for operation in ["query", "fragment", "reload"] {
        for call in [
            "top",
            "child",
            "borrow-child",
            "borrow-top",
            "popup",
            "borrow-popup",
        ] {
            let mut page = SameDocumentPage::new().await;
            let script = r###"(async () => {
  const operation = OPERATION, call = CALL;
  const tick = () => new Promise(resolve => setTimeout(resolve, 0));
  async function frame(w, name, shadow = false) {
    const element = w.document.createElement('iframe');
    const loaded = new Promise(resolve => element.onload = resolve);
    element.src = 'history.html?' + name;
    let root = w.document.body;
    if (shadow) {
      const host = w.document.createElement('div');
      root.appendChild(host);
      root = host.attachShadow({mode: 'closed'});
    }
    root.appendChild(element);
    await loaded;
    return element.contentWindow;
  }
  const child = await frame(window, 'child');
  const sibling = await frame(window, 'sibling');
  const grandchild = await frame(child, 'grandchild');
  const shadow = await frame(window, 'shadow', true);
  const popup = open('history.html?popup');
  await new Promise(resolve => popup.addEventListener('load', resolve, {once: true}));
  await tick();
  const windows = {top: window, child, sibling, grandchild, shadow, popup};
  const checks = {}, records = {}, events = [];
  for (const [name, w] of Object.entries(windows)) {
    const nav = w.navigation;
    const record = records[name] = {
      nav, before: w.location.href, entry: nav.currentEntry,
      length: nav.entries().length, states: {}, handlers: 0, errors: 0, successes: 0,
    };
    const observe = (name, promise) => promise.then(
      () => record.states[name] = 'fulfilled',
      error => { record.states[name] = error.name; record.reasons.push(error); },
    );
    record.reasons = [];
    nav.addEventListener('navigateerror', event => {
      record.errors++; record.reasons.push(event.error); events.push('error:' + name);
    });
    nav.addEventListener('navigatesuccess', () => record.successes++);
    nav.addEventListener('navigate', event => {
      record.signal = event.signal;
      event.signal.addEventListener('abort', () => events.push('abort:' + name));
      event.intercept({
        precommitHandler() {
          record.transition = nav.transition;
          record.observed = [
            observe('transition committed', nav.transition.committed),
            observe('transition finished', nav.transition.finished),
          ];
          return new Promise(resolve => record.release = resolve);
        },
        handler() { record.handlers++; },
      });
    }, {once: true});
    const result = operation === 'reload' ? nav.reload()
      : nav.navigate(operation === 'fragment' ? '#requested' : '?requested');
    record.observed.push(observe('method committed', result.committed), observe('method finished', result.finished));
    // Public frame properties must not determine the native cancellation scope.
    Object.defineProperty(w, 'frames', {configurable: true, value: {get length() {throw new Error('frames accessed');}}});
  }
  await tick();
  const stopped = call.includes('popup') ? ['popup']
    : call.includes('child') ? ['child', 'grandchild']
    : ['top', 'child', 'sibling', 'grandchild', 'shadow'];
  const stop = () => {
    if (call === 'top') window.stop();
    else if (call === 'child') child.stop();
    else if (call === 'borrow-child') window.stop.call(child);
    else if (call === 'borrow-top') child.stop.call(window);
    else if (call === 'popup') popup.stop();
    else child.stop.call(popup);
  };
  stop(); stop();
  for (const [name, record] of Object.entries(records)) {
    const canceled = stopped.includes(name), w = windows[name];
    checks[name + ' synchronous abort'] = record.signal.aborted === canceled;
    checks[name + ' synchronous transition'] = canceled ? record.nav.transition === null : record.nav.transition === record.transition;
    checks[name + ' retained entry'] = w.location.href === record.before && w.document.URL === record.before
      && record.nav.currentEntry === record.entry && record.nav.entries().length === record.length;
  }
  await tick();
  for (const record of Object.values(records)) record.release();
  checks.settled = await Promise.race([
    Promise.all(Object.values(records).flatMap(record => record.observed)).then(() => true),
    new Promise(resolve => setTimeout(() => resolve(false), 900)),
  ]);
  await tick();
  for (const [name, record] of Object.entries(records)) {
    const canceled = stopped.includes(name), w = windows[name];
    checks[name + ' promise states'] = Object.keys(record.states).length === 4
      && Object.values(record.states).every(value => value === (canceled ? 'AbortError' : 'fulfilled'));
    checks[name + ' events'] = record.errors === (canceled ? 1 : 0) && record.successes === (canceled ? 0 : 1);
    checks[name + ' handler'] = record.handlers === (canceled ? 0 : 1);
    checks[name + ' transition cleared'] = record.nav.transition === null;
    checks[name + ' error identity'] = !canceled || record.reasons.length === 5
      && record.reasons.every(error => error === record.signal.reason && error instanceof w.DOMException);
    checks[name + ' stopped history'] = !canceled || w.location.href === record.before
      && w.document.URL === record.before && record.nav.currentEntry === record.entry && record.nav.entries().length === record.length;
  }
  popup.close();
  return {checks, events, states: Object.fromEntries(Object.entries(records).map(([name, record]) => [name, record.states]))};
})()"###
                .replace("OPERATION", &json!(operation).to_string())
                .replace("CALL", &json!(call).to_string());
            let result = page.evaluate(&script).await;
            assert!(
                result["checks"]
                    .as_object()
                    .unwrap()
                    .values()
                    .all(|value| value == true),
                "{operation}/{call}: {result}"
            );
            if matches!(call, "top" | "borrow-top") {
                page.assert_history(&[""], 0).await;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn asynchronous_precommit_controllers_apply_final_redirects() {
    for (owner, operation, mode) in [
        ("top", "navigate", "push"),
        ("top", "navigate", "replace"),
        ("top", "download", "push"),
        ("top", "download", "replace"),
        ("top", "location", "push"),
        ("top", "location", "replace"),
        ("top", "reload", "add"),
        ("top", "back", "add"),
        ("child", "navigate", "push"),
        ("child", "navigate", "replace"),
        ("child", "download", "push"),
        ("child", "download", "replace"),
        ("child", "location", "push"),
        ("child", "location", "replace"),
        ("child", "reload", "add"),
        ("popup", "navigate", "push"),
        ("popup", "navigate", "replace"),
        ("popup", "download", "push"),
        ("popup", "download", "replace"),
        ("popup", "location", "push"),
        ("popup", "location", "replace"),
        ("popup", "reload", "add"),
        ("top", "navigate", "cancel-stop"),
        ("top", "navigate", "cancel-navigate"),
        ("top", "navigate", "reject"),
        ("top", "download", "cancel-stop"),
        ("top", "download", "cancel-navigate"),
        ("top", "download", "reject"),
        ("top", "location", "cancel-stop"),
        ("top", "location", "cancel-navigate"),
        ("top", "location", "reject"),
        ("top", "navigate-cross-document", "push"),
        ("top", "navigate-cross-document", "replace"),
        ("top", "location-cross-document", "push"),
        ("top", "location-cross-document", "replace"),
        ("top", "anchor", "push"),
        ("top", "anchor", "replace"),
        ("top", "form", "push"),
        ("top", "form", "replace"),
        ("top", "navigate", "document-base"),
        ("top", "download", "document-base"),
        ("top", "navigate", "parallel"),
    ] {
        let mut page = SameDocumentPage::new().await;
        page.allow_downloads().await;
        let script = r###"(async () => {
  const ownerKind = OWNER, operation = OPERATION, mode = MODE;
  const parentURL = location.href;
  let target = window, frame;
  if (ownerKind === 'child') {
    frame = document.createElement('iframe'); frame.src = 'history.html'; document.body.appendChild(frame);
    await new Promise(resolve => frame.addEventListener('load', resolve, {once:true})); target = frame.contentWindow;
  } else if (ownerKind === 'popup') {
    target = open('history.html'); await new Promise(resolve => target.addEventListener('load', resolve, {once:true}));
  }
  const result = await (async () => {
    const nav = target.navigation, loc = target.location, doc = target.document;
    const tick = () => new Promise(resolve => target.setTimeout(resolve, 0));
    if (operation === 'back') target.history.pushState(null, '', '#setup');
    await tick();
    const before = loc.href, entriesBefore = nav.entries().map(e => e.url), from = nav.currentEntry;
    const checks = {}, order = [], states = {}, failures = [], transitionResults = [];
    const canceled = mode.startsWith('cancel'), rejected = mode === 'reject';
    const expectedError = new Error('precommit rejected');
    let release, event, controller, transition, method, nested, addHandlerSupported;
    let lateHandlerCalls = 0;
    const controllerErrors = [];
    const gate = new Promise(resolve => release = resolve);
    const state = {step:1}, info = {phase:1};
    const expectedKind = mode === 'replace' ? 'replace' : 'push';
    const redirectFirst = mode === 'document-base' ? 'first#step' : '?redirect=1#first';
    const redirectFinal = mode === 'document-base' ? 'final#final' : '?redirect=2#final';
    if (mode === 'document-base') { const base = doc.createElement('base'); base.href='/redirect-base/';doc.head.appendChild(base); }
    const finalURL = new URL(redirectFinal, mode === 'document-base' ? new URL('/redirect-base/', before).href : before).href;
    const invalid = (phase, callback) => {
      try { callback(); checks[phase] = false; }
      catch(error) { controllerErrors.push({phase,name:error?.name}); checks[phase] = error.name === 'InvalidStateError'; }
    };
    const checkController = phase => {
      if (canceled || rejected) {
        try {
          controller.redirect('#retained-event');
          checks[phase+' redirect'] = event.destination.url.endsWith('#retained-event');
        } catch { checks[phase+' redirect'] = false; }
        if (addHandlerSupported) {
          try { controller.addHandler(() => lateHandlerCalls++); checks[phase+' addHandler'] = true; }
          catch { checks[phase+' addHandler'] = false; }
        }
      } else {
        invalid(phase+' redirect', () => controller.redirect('#invalid'));
        if (addHandlerSupported) invalid(phase+' addHandler', () => controller.addHandler(() => {}));
      }
    };
    const observe = (name, promise) => promise.then(value => {
      states[name] = 'fulfilled'; return value;
    }, reason => { states[name] = 'rejected'; failures.push({name,kind:reason?.name,identity:reason===expectedError}); return reason; });
    nav.addEventListener('currententrychange', () => {
      if (!event || event.signal.aborted) return;
      order.push('currententrychange'); checkController('at commit');
    });
    nav.addEventListener('navigatesuccess', () => { if (!event.signal.aborted) order.push('success'); });
    nav.addEventListener('navigateerror', error => {
      order.push('error'); checks['error reason'] = canceled ? error.error === event.signal.reason && error.error.name === 'AbortError' : error.error === expectedError;
    }, {once:true});
    nav.addEventListener('navigate', e => {
      event = e;
      e.intercept({async precommitHandler(c) {
        controller = c; addHandlerSupported = typeof c.addHandler === 'function'; checks['addHandler supported'] = addHandlerSupported; order.push('precommit'); transition = nav.transition;
        checks['transition identity'] = transition !== null && transition.from === from && transition.to === e.destination;
        if (transition) {
          transitionResults.push(observe('committed', transition.committed));
          transitionResults.push(observe('finished', transition.finished));
        }
        await gate;
        if (canceled) { checkController('after cancel'); return; }
        if (rejected) throw expectedError;
        checks['still pending after await'] = loc.href === before && nav.currentEntry === from && states.committed === undefined;
        if (operation === 'reload' || operation === 'back') {
          invalid('non-redirectable type', () => c.redirect('#invalid'));
        } else {
          c.redirect(redirectFirst, {history: expectedKind === 'push' ? 'replace' : 'push', state, info});
          order.push('redirect1'); state.step = 2; info.phase = 2;
          await tick();
          c.redirect(redirectFinal, {history: expectedKind, state, info});
          order.push('redirect2'); state.step = 99;
          checks['redirect destination'] = e.destination.url === finalURL && e.destination.getState().step === 2 && e.info === info;
          checks['redirect navigation type'] = e.navigationType === expectedKind;
        }
        if (addHandlerSupported) c.addHandler(() => {order.push('added'); checkController('added handler');});
      }, handler() {
        order.push('handler'); checkController('handler');
        checks['commit visible in handler'] = doc.URL === loc.href && nav.currentEntry.url === loc.href;
      }});
    }, {once:true});
    if (mode === 'parallel') nav.addEventListener('navigate', e => e.intercept({async precommitHandler(c) {
      await gate; await tick(); await tick();
      c.redirect(finalURL);
      checks['parallel precommit remains active'] = loc.href === before && !order.includes('handler');
      if (typeof c.addHandler === 'function') c.addHandler(() => order.push('parallel added'));
    }}), {once:true});
    if (operation.startsWith('navigate')) method = nav.navigate(new URL(operation === 'navigate' && mode !== 'cancel-stop' ? '#original' : '?requested=1#original',before).href, {state:{initial:true}});
    else if (operation.startsWith('location')) loc.assign(operation === 'location' && mode !== 'cancel-stop' ? '#original' : '?requested=1#original');
    else if (operation === 'anchor') { const a = doc.createElement('a'); a.href='?requested=1#original'; doc.body.appendChild(a); a.click(); }
    else if (operation === 'form') { const form=doc.createElement('form');form.action='?requested=1#original';form.method='post';doc.body.appendChild(form);form.requestSubmit(); }
    else if (operation === 'download') {
      const a = doc.createElement('a'); a.href = '?download=1#original'; a.download = 'example.txt'; doc.body.appendChild(a); a.click();
    } else method = nav[operation]();
    const methodResults = method ? [observe('method committed',method.committed),observe('method finished',method.finished)] : [];
    for (let i=0; i<50 && !controller && states['method committed'] === undefined; i++) await tick();
    checks['precommit started'] = controller !== undefined;
    await tick();
    checks['pending before release'] = loc.href === before && nav.currentEntry === from && !order.includes('handler') && states.committed === undefined;
    if (canceled) {
      if (mode === 'cancel-stop') target.stop();
      else nested = nav.navigate('#next');
    }
    release();
    const all = Promise.all([...transitionResults,...methodResults]);
    const settled = await Promise.race([all, new Promise(resolve => target.setTimeout(() => resolve(null),700))]);
    checks['settled'] = settled !== null && transitionResults.length === 2;
    if (nested) await nested.finished;
    await tick();
    checks['transition cleared'] = nav.transition === null;
    checkController('after finish');
    await tick();
    if (canceled || rejected) {
      checks['canceled handlers stay canceled'] = lateHandlerCalls === 0;
      checks['rejected before commit'] = states.committed === 'rejected' && states.finished === 'rejected' && !order.includes('handler') && !order.includes('added');
      checks['no late commit'] = loc.href === (nested ? new URL('#next',before).href : before);
    } else {
      checks['promise values'] = settled !== null && settled[0] === undefined && settled[1] === undefined && (!method || settled[2]===nav.currentEntry && settled[3]===nav.currentEntry);
      checks['handler order'] = order.indexOf('currententrychange') < order.indexOf('handler') && (addHandlerSupported ? order.indexOf('handler') < order.indexOf('added') && order.indexOf('added') < order.indexOf('success') : order.indexOf('handler') < order.indexOf('success'));
      if (operation !== 'reload' && operation !== 'back') {
        const expectedEntries = mode === 'replace' ? [...entriesBefore.slice(0,-1),finalURL] : [...entriesBefore,finalURL];
        checks['final history'] = JSON.stringify(nav.entries().map(e=>e.url)) === JSON.stringify(expectedEntries) && loc.href === finalURL && doc.URL === finalURL;
        checks['entry state'] = operation.startsWith('navigate') ? nav.currentEntry.getState()?.step === 2 : nav.currentEntry.getState() === undefined;
      }
    }
    return {ownerKind,operation,mode,addHandlerSupported,promiseValues:settled?.map(v=>({undefined:v===undefined,entry:v===nav.currentEntry,name:v?.name})),controllerErrors,checks,order,states,failures,href:loc.href,entries:nav.entries().map(e=>e.url),transitionType:transition?.navigationType,eventType:event?.navigationType};
  })();
  if (ownerKind !== 'top') { result.checks['parent unchanged'] = location.href === parentURL && navigation.transition === null; if(frame)frame.remove();else target.close(); }
  return result;
})()
"###
            .replace("OWNER", &json!(owner).to_string())
            .replace("OPERATION", &json!(operation).to_string())
            .replace("MODE", &json!(mode).to_string());
        let result = page.evaluate(&script).await;
        assert!(
            result["checks"]
                .as_object()
                .unwrap()
                .values()
                .all(|value| value == true),
            "{owner}/{operation}/{mode}: {result}"
        );
        assert_eq!(
            page.download_requests
                .load(std::sync::atomic::Ordering::SeqCst),
            0,
            "intercepted download must not be fetched"
        );
        if owner == "top" {
            if mode == "document-base" {
                let browser = page.command("Page.getNavigationHistory", json!({})).await;
                let urls: Vec<_> = browser["entries"].as_array().unwrap()[page.browser_base..]
                    .iter()
                    .map(|entry| entry["url"].clone())
                    .collect();
                assert_eq!(json!(urls), result["entries"]);
                assert_eq!(browser["currentIndex"], json!(page.browser_base + 1));
            } else {
                match (operation, mode) {
                    (_, "cancel-navigate") => page.assert_history(&["", "#next"], 1).await,
                    (_, "cancel-stop" | "reject") | ("reload", _) => {
                        page.assert_history(&[""], 0).await
                    }
                    ("back", _) => page.assert_history(&["", "#setup"], 0).await,
                    (_, "replace") => page.assert_history(&["?redirect=2#final"], 0).await,
                    _ => page.assert_history(&["", "?redirect=2#final"], 1).await,
                }
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn asynchronous_precommit_controller_rejects_retired_windows() {
    for owner in ["child", "popup"] {
        let mut page = SameDocumentPage::new().await;
        let script = r###"(async () => {
  const ownerKind = OWNER;
  const before = location.href;
  let target, frame, controller, release;
  const tick = () => new Promise(resolve => setTimeout(resolve,0));
  if (ownerKind === 'child') {
    frame=document.createElement('iframe');frame.src='history.html';document.body.appendChild(frame);
    await new Promise(resolve=>frame.addEventListener('load',resolve,{once:true}));target=frame.contentWindow;
  } else {
    target=open('history.html');await new Promise(resolve=>target.addEventListener('load',resolve,{once:true}));
  }
  await tick();
  target.navigation.addEventListener('navigate',event=>event.intercept({precommitHandler(c){controller=c;return new Promise(resolve=>release=resolve);}}),{once:true});
  const method=target.navigation.navigate(new URL('#pending',target.location.href).href);
  method.committed.catch(()=>{});method.finished.catch(()=>{});
  for(let i=0;i<50&&!controller;i++) await tick();
  const checks={'precommit started':controller!==undefined};
  if(frame)frame.remove();else target.close();
  for(const [name,invoke] of [['redirect',()=>controller.redirect('#late')],['addHandler',()=>controller.addHandler(()=>{})]]) {
    try { invoke();checks[name+' after retirement']=false; }
    catch(error) { checks[name+' after retirement']=error.name==='InvalidStateError'; }
  }
  release?.();await tick();
  checks['parent unchanged']=location.href===before&&navigation.transition===null;
  return {ownerKind,checks};
})()
"###.replace("OWNER", &json!(owner).to_string());
        let result = page.evaluate(&script).await;
        assert!(
            result["checks"]
                .as_object()
                .unwrap()
                .values()
                .all(|value| value == true),
            "{owner}: {result}"
        );
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

#[tokio::test(flavor = "multi_thread")]
async fn popup_close_cancels_intercepted_navigations_before_retiring_the_window() {
    for operation in ["query", "fragment", "reload"] {
        for phase in ["precommit", "handler"] {
            for late in ["resolve", "reject"] {
                let mut page = SameDocumentPage::new().await;
                let script = r###"(async () => {
 const operation=OPERATION, phase=PHASE, late=LATE;
 const tick=()=>new Promise(r=>setTimeout(r,0));
 const w=open('history.html?popup-close','closing-window');
 await new Promise(r=>w.addEventListener('load',r,{once:true}));await tick();
 const nav=w.navigation, Ctor=w.DOMException, events=[],states={},reasons=[],checks={};
 let signal,transition,release,reject,handlers=0;
 const observe=(name,p)=>p.then(()=>states[name]='fulfilled',e=>{states[name]=e.name;reasons.push(e);});
 nav.addEventListener('navigateerror',e=>{events.push('error:'+e.error.name);reasons.push(e.error);});
 nav.addEventListener('navigatesuccess',()=>events.push('success'));
 nav.addEventListener('navigate',e=>{
  signal=e.signal;e.signal.addEventListener('abort',()=>events.push('abort'));
  const hold=()=>{transition=nav.transition;observe('transitionCommitted',transition.committed);observe('transitionFinished',transition.finished);return new Promise((resolve,fail)=>{release=resolve;reject=fail;});};
  e.intercept(phase==='precommit'?{precommitHandler:hold,handler(){handlers++;}}:{handler(){handlers++;return hold();}});
 },{once:true});
 const result=operation==='reload'?nav.reload():nav.navigate(operation==='fragment'?'#pending':'?pending');
 observe('committed',result.committed);observe('finished',result.finished);
 w.close();w.close();
 await new Promise(r=>setTimeout(r,100));
 const afterClose={closed:w.closed,aborted:signal.aborted,events:[...events],states:{...states},transitionCleared:nav.transition===null};
 if(late==='resolve')release();else reject(new Error('late rejection'));
 await new Promise(r=>setTimeout(r,50));
 const afterRelease={closed:w.closed,aborted:signal.aborted,events:[...events],states:{...states},transitionCleared:nav.transition===null};
 checks.closed=w.closed;checks.aborted=signal.aborted;checks.noLateCompletion=JSON.stringify(afterClose)===JSON.stringify(afterRelease);
 checks.events=events.join(',')==='abort,error:AbortError';checks.transitionCleared=nav.transition===null;
 checks.promises=Object.keys(states).length===4&&Object.entries(states).every(([name,value])=>value===(phase==='handler'&&name.toLowerCase().endsWith('committed')?'fulfilled':'AbortError'));
 checks.errorIdentity=reasons.length===(phase==='precommit'?5:3)&&reasons.every(e=>e===signal.reason&&e instanceof Ctor);
 checks.handlers=handlers===(phase==='precommit'?0:1);
 return {checks,afterClose,afterRelease,handlers,reasons:reasons.map(e=>e.name)};
})()"###
                    .replace("OPERATION", &json!(operation).to_string())
                    .replace("PHASE", &json!(phase).to_string())
                    .replace("LATE", &json!(late).to_string());
                let result = page.evaluate(&script).await;
                assert!(
                    result["checks"]
                        .as_object()
                        .unwrap()
                        .values()
                        .all(|value| value == true),
                    "{operation}/{phase}/{late}: {result}"
                );
                page.assert_history(&[""], 0).await;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_close_preserves_other_window_owners_during_cancellation_callbacks() {
    for phase in ["precommit", "handler"] {
        for action in [
            "repeat-close",
            "navigate",
            "open-same-name",
            "close-sibling",
            "detach-opener",
            "retired-opener",
        ] {
            let mut page = SameDocumentPage::new().await;
            let script = r###"(async () => {
 const action=ACTION, phase=PHASE;
 const tick=()=>new Promise(r=>setTimeout(r,0));
 let openerFrame;
 let opener=window;
 if(['detach-opener','retired-opener'].includes(action)){
  openerFrame=document.createElement('iframe');openerFrame.src='history.html?opener';
  const loaded=new Promise(r=>openerFrame.onload=r);document.body.appendChild(openerFrame);await loaded;
  opener=openerFrame.contentWindow;
 }
 async function popup(owner,url,name){const w=owner.open(url,name);await new Promise(r=>w.addEventListener('load',r,{once:true}));await tick();return w;}
 const closing=await popup(opener,'history.html?closing','closing-window');
 const sibling=await popup(window,'history.html?sibling','sibling-window');
 const records={},events=[],checks={},extra={};let replacement,replacementLoaded;
 function arm(name,w,stage){
  const nav=w.navigation,record=records[name]={w,nav,Ctor:w.DOMException,states:{},reasons:[],handlers:0};
  const observe=(key,p)=>p.then(()=>record.states[key]='fulfilled',e=>{record.states[key]=e.name;record.reasons.push(e);});
  nav.addEventListener('navigateerror',e=>{events.push('error:'+name);record.reasons.push(e.error);});
  nav.addEventListener('navigatesuccess',()=>events.push('success:'+name));
  nav.addEventListener('navigate',e=>{
   record.signal=e.signal;
   e.signal.addEventListener('abort',()=>{
    events.push('abort:'+name);
    if(name!=='closing')return;
    extra.callbackClosed=closing.closed;
    if(action==='repeat-close'){closing.close();closing.stop();}
    if(action==='navigate'){
     const result=nav.navigate('#reentrant');
     result.committed.then(()=>extra.committed='fulfilled',e=>extra.committed=e.name);
     result.finished.then(()=>extra.finished='fulfilled',e=>extra.finished=e.name);
    }
    if(action==='open-same-name'){
     replacement=open('history.html?replacement','closing-window');
     extra.distinct=replacement!==closing;
     replacementLoaded=new Promise(r=>replacement.addEventListener('load',r,{once:true}));
    }
    if(action==='close-sibling')sibling.close();
    if(action==='detach-opener')openerFrame.remove();
   });
   const hold=()=>{record.transition=nav.transition;observe('transitionCommitted',nav.transition.committed);observe('transitionFinished',nav.transition.finished);return new Promise(r=>record.release=r);};
   e.intercept(stage==='precommit'?{precommitHandler:hold,handler(){record.handlers++;}}:{handler(){record.handlers++;return hold();}});
  },{once:true});
  const result=nav.navigate('#pending-'+name);observe('committed',result.committed);observe('finished',result.finished);
 }
 arm('opener',window,'precommit');arm('closing',closing,phase);arm('sibling',sibling,'precommit');
 await tick();if(action==='retired-opener')openerFrame.remove();closing.close();
 await new Promise(r=>setTimeout(r,150));
 checks.closed=closing.closed;
 for(const [name,r]of Object.entries(records)){
  const canceled=name==='closing'||name==='sibling'&&action==='close-sibling';
  checks[name+' abort scope']=r.signal.aborted===canceled;
  checks[name+' pending control']=canceled||Object.keys(r.states).length===0;
 }
 if(replacement){
  checks.replacementLoaded=await Promise.race([replacementLoaded.then(()=>true),new Promise(r=>setTimeout(()=>r(false),1000))]);
  checks.replacementOpen=!replacement.closed&&replacement!==closing;
  if(checks.replacementLoaded){arm('replacement',replacement,'precommit');await tick();}
 }
 for(const r of Object.values(records))r.release();
 await new Promise(r=>setTimeout(r,50));
 for(const [name,r]of Object.entries(records)){
  const canceled=name==='closing'||name==='sibling'&&action==='close-sibling';
  checks[name+' states']=Object.keys(r.states).length===4&&Object.entries(r.states).every(([key,value])=>value===(canceled&&!(name==='closing'&&phase==='handler'&&key.toLowerCase().endsWith('committed'))?'AbortError':'fulfilled'));
  checks[name+' transition']=r.nav.transition===null;
  checks[name+' terminal events']=events.filter(e=>e==='success:'+name).length===(canceled?0:1)&&events.filter(e=>e==='error:'+name).length===(canceled?1:0)&&events.filter(e=>e==='abort:'+name).length===(canceled?1:0);
  checks[name+' identity']=!canceled||r.reasons.length===(name==='closing'&&phase==='handler'?3:5)&&r.reasons.every(e=>e===r.signal.reason&&e instanceof r.Ctor);
 }
 if(action==='navigate')checks.reentrantNavigation=extra.committed===extra.finished&&['AbortError','InvalidStateError'].includes(extra.committed);
 sibling.close();if(replacement)replacement.close();if(openerFrame)openerFrame.remove();
 return {checks,events,extra,states:Object.fromEntries(Object.entries(records).map(([name,r])=>[name,r.states]))};
})()"###
                .replace("PHASE", &json!(phase).to_string())
                .replace("ACTION", &json!(action).to_string());
            let result = page.evaluate(&script).await;
            assert!(
                result["checks"]
                    .as_object()
                    .unwrap()
                    .values()
                    .all(|value| value == true),
                "{phase}/{action}: {result}"
            );
        }
    }
}
