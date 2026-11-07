use super::*;
use crate::runtime::RendererBrowserContextRuntime;
use crate::worker::WorkerGlobalKind;
use tokio::io::AsyncReadExt;

struct RegistrationServer {
    origin: String,
    task: JoinHandle<()>,
}

impl Drop for RegistrationServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn registration_server() -> RegistrationServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut chunk = [0; 2048];
                loop {
                    let length = socket.read(&mut chunk).await.unwrap();
                    if length == 0 {
                        return;
                    }
                    request.extend_from_slice(&chunk[..length]);
                    if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        break;
                    }
                }
                let line = String::from_utf8_lossy(&request);
                let path = line.split_whitespace().nth(1).unwrap();
                let body = if path == "/actual-sw.js" {
                    "self.addEventListener('install', event => event.waitUntil(Promise.resolve()));"
                } else {
                    "throw new Error('Unexpected script URL');"
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nService-Worker-Allowed: /\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    RegistrationServer { origin, task }
}

async fn registration_probe(
    options: WorkerSpawnOptions,
    browser: &RendererBrowserContextRuntime,
) -> serde_json::Value {
    let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
    browser.add_service_worker_owner_wake_sender(wake_tx);
    let mut worker = spawn_test_worker_with_options(options);
    let value = loop {
        let message = timeout(TIMEOUT, async {
            loop {
                browser.drain_service_worker_service_lane();
                tokio::select! {
                    message = worker.recv() => break message,
                    _ = wake_rx.recv() => {}
                }
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "worker registration probe timed out: {:?}",
                browser.service_worker_runtime().diagnostics_snapshot()
            )
        })
        .expect("worker exited before reporting");
        match message {
            WorkerToParentMessage::Console(message) => {
                let text = message.message.strip_prefix("log: ").unwrap_or_else(|| {
                    panic!("unexpected console diagnostic: {}", message.message)
                });
                assert!(!text.starts_with("FAIL:"), "{text}");
                break serde_json::from_str(text)
                    .unwrap_or_else(|error| panic!("worker probe JSON {error}: {text}"));
            }
            WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
            other => panic!("unexpected registration probe output: {other:?}"),
        }
    };
    worker.terminate_and_join();
    value
}

#[tokio::test]
async fn worker_service_worker_container_registers_real_jobs_and_delivers_trusted_types_and_lifecycle()
 {
    ensure_v8();
    let server = registration_server().await;
    let url = url::Url::parse(&format!("{}/worker/main.js", server.origin)).unwrap();
    for kind in [
        WorkerGlobalKind::Dedicated {
            name: "registration".into(),
        },
        WorkerGlobalKind::Shared {
            name: "registration".into(),
            storage_key: moli_storage_key::MoliStorageKey::first_party_from_url(&url, None),
        },
        WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(90001),
            version_id: ServiceWorkerVersionId::from_u64_for_test(90001),
            scope_url: url.join("./").unwrap(),
        },
    ] {
        let browser = RendererBrowserContextRuntime::new();
        let source = r#"
(async () => {
  const check = (ok, label) => { if (!ok) throw new Error(label); };
  const container = navigator.serviceWorker;
  const policy = trustedTypes.createPolicy('named', {createScriptURL: value => value.replace('wrong-sw', 'actual-sw')});
  const violation = new Promise(resolve => addEventListener('securitypolicyviolation', resolve, {once:true}));
  let blocked;
  try { await container.register('../wrong-sw.js', {scope:'../blocked/'}); }
  catch (error) { blocked = error instanceof TypeError; }
  check(blocked, 'plain string rejected');
  const report = await violation;
  check(report.sample === 'ServiceWorkerContainer register|../wrong-sw.js', 'Trusted Types sink report');
  const registrations = [];
  const installing = [];
  const order = [];
  for (const name of ['one', 'two']) {
    if (name === 'two') trustedTypes.createPolicy('default', {createScriptURL(value, type, sink) {
      check(type === 'TrustedScriptURL' && sink === 'ServiceWorkerContainer register', 'default policy arguments');
      return value.replace('wrong-sw', 'actual-sw');
    }});
    const value = name === 'one' ? policy.createScriptURL('../wrong-sw.js#ignored') : '../wrong-sw.js#ignored';
    const reg = await container.register(value, {scope:'../'+name+'/', updateViaCache:'none'});
    check(reg instanceof ServiceWorkerRegistration, 'registration realm');
    check(reg.updateViaCache === 'none', 'updateViaCache');
    check(reg.scope === new URL('../'+name+'/', location.href).href, 'scope uses worker settings');
    await new Promise(resolve => reg.addEventListener('updatefound', () => {
      order.push(name+' updatefound');
      check(reg.installing instanceof ServiceWorker && reg.installing.state === 'installing', 'installing task snapshot');
      installing.push(reg.installing);
      const sw = reg.installing;
      check(sw.scriptURL === new URL('../actual-sw.js', location.href).href, 'trusted URL transformation and fragment removal');
      sw.addEventListener('statechange', () => {
        order.push(name+' '+sw.state);
        if (sw.state === 'activated') resolve();
      });
    }, {once:true}));
    check(reg.active === installing.at(-1), 'version wrapper identity');
    check(await container.getRegistration('../'+name+'/client#fragment') === reg, 'getRegistration identity');
    registrations.push(reg);
  }
  check(registrations[0] !== registrations[1], 'distinct registration wrappers');
  check(registrations[0].active !== registrations[1].active, 'distinct version wrappers');
  const list = await container.getRegistrations();
  check(list.length === 2 && Object.isFrozen(list), 'getRegistrations FrozenArray');
  let crossOrigin;
  try { await container.getRegistration('https://other.test/'); } catch (e) { crossOrigin = e.name; }
  check(crossOrigin === 'SecurityError', 'query origin restriction');
  const results = [];
  for (const reg of registrations) {
    results.push(await reg.unregister(), await reg.unregister());
    check(await container.getRegistration(reg.scope) === undefined, 'unregistered lookup');
  }
  check((await container.getRegistrations()).length === 0, 'unregistered list');
  console.log(JSON.stringify({blocked, sample:report.sample, results, order}));
})().catch(error => console.log('FAIL:'+error.stack));
"#;
        let actual = registration_probe(
            WorkerSpawnOptions::new(source.into(), url.to_string())
                .with_global_kind(kind)
                .with_worker_context_runtime(browser.worker_context_runtime())
                .with_service_worker_runtime(browser.service_worker_runtime())
                .with_content_security_policies(vec!["require-trusted-types-for 'script'".into()]),
            &browser,
        )
        .await;
        assert_eq!(actual["blocked"], true);
        assert_eq!(
            actual["sample"],
            "ServiceWorkerContainer register|../wrong-sw.js"
        );
        assert_eq!(
            actual["results"],
            serde_json::json!([true, false, true, false])
        );
        assert_eq!(
            actual["order"],
            serde_json::json!([
                "one updatefound",
                "one installed",
                "one activating",
                "one activated",
                "two updatefound",
                "two installed",
                "two activating",
                "two activated",
                "one redundant",
                "two redundant",
            ])
        );
    }
}

#[tokio::test]
async fn worker_service_worker_container_registration_uses_the_worker_storage_partition() {
    ensure_v8();
    let server = registration_server().await;
    let url = url::Url::parse(&format!("{}/worker/main.js", server.origin)).unwrap();
    let browser = RendererBrowserContextRuntime::new();
    for site in ["https://first.test", "https://second.test"] {
        let source = r#"
(async () => {
  const container = navigator.serviceWorker;
  const before = (await container.getRegistrations()).length;
  const missing = await container.getRegistration('../partition/child') === undefined;
  const reg = await container.register('../actual-sw.js', {scope:'../partition/'});
  await new Promise(resolve => reg.addEventListener('updatefound', resolve, {once:true}));
  const same = await container.getRegistration('../partition/child') === reg;
  console.log(JSON.stringify({before,missing,same,after:(await container.getRegistrations()).length}));
})().catch(error => console.log('FAIL:'+error.stack));
"#;
        let actual = registration_probe(
            WorkerSpawnOptions::new(source.into(), url.to_string())
                .with_api_storage_key(Some(
                    moli_storage_key::MoliStorageKey::from_url_and_top_level_site(
                        &url,
                        site.into(),
                        None,
                    ),
                ))
                .with_storage_key_top_level_site(Some(site.into()))
                .with_worker_context_runtime(browser.worker_context_runtime())
                .with_service_worker_runtime(browser.service_worker_runtime()),
            &browser,
        )
        .await;
        assert_eq!(
            actual,
            serde_json::json!({"before":0,"missing":true,"same":true,"after":1})
        );
    }
    assert_eq!(
        browser
            .service_worker_runtime()
            .diagnostics_snapshot()
            .registration_count,
        2
    );
}
