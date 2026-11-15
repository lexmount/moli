use super::{clean_output, fixture_runtime, run_moli};
use anyhow::Result;
use axum::{
    Router,
    body::Bytes,
    http::{HeaderMap, Method},
    response::{Html, Redirect},
    routing::{any, get},
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::sync::Arc;

const READ: &str = r#"
async function read(url, api, init = {}) {
  if (api === 'fetch') {
    const request = new Request(url, init);
    const response = await fetch(request);
    return {body: await response.text(), header: response.headers.get('x-secret'),
            cookie: response.headers.get('set-cookie'), mode: request.mode,
            redirected: response.redirected};
  }
  const xhr = new XMLHttpRequest();
  xhr.open(init.method || 'GET', url, api !== 'sync-xhr');
  for (const [name, value] of Object.entries(init.headers || {})) xhr.setRequestHeader(name, value);
  const result = () => ({body: xhr.responseText, header: xhr.getResponseHeader('x-secret'),
                        cookie: xhr.getResponseHeader('set-cookie')});
  if (api === 'sync-xhr') { xhr.send(init.body || null); return result(); }
  return new Promise((resolve, reject) => {
    xhr.onload = () => resolve(result());
    xhr.onerror = () => reject(new Error('XHR network error'));
    xhr.send(init.body || null);
  });
}
"#;

#[derive(Debug)]
struct Seen {
    method: Method,
    headers: HeaderMap,
    body: Bytes,
}

struct Fixture {
    _runtime: tokio::runtime::Runtime,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    origin: String,
    target: String,
    requests: Arc<Mutex<Vec<Seen>>>,
}

impl Fixture {
    fn new() -> Result<Self> {
        let runtime = fixture_runtime()?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let logged = requests.clone();
        let (origin, target, tasks) = runtime.block_on(async move {
            let target = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let target_url = format!("http://{}", target.local_addr()?);
            let target_app = Router::new()
                .route("/target", any(move |method: Method, headers: HeaderMap, body: Bytes| {
                    let logged = logged.clone();
                    async move {
                        logged.lock().push(Seen { method, headers, body });
                        ([("x-secret", "visible"), ("set-cookie", "hidden=1; Path=/")], "payload")
                    }
                }))
                .route("/redirect", get(|| async { Redirect::temporary("/target") }))
                .route("/frame", get(|| async { Html("<p>cross-origin document</p>") }));
            let target_task = tokio::spawn(async move { axum::serve(target, target_app).await.unwrap() });

            let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let origin_url = format!("http://{}", origin.local_addr()?);
            let frame = format!("<iframe src='{target_url}/frame'></iframe>");
            let worker = format!("{READ}; onmessage = async e => {{ try {{ postMessage(await read(e.data.url, e.data.api)); }} catch(error) {{ postMessage({{error:error.message, name:error.name}}); }} }}");
            let origin_app = Router::new()
                .route("/", get(|| async { Html("<main>ready</main>") }))
                .route("/csp", get(|| async { ([("content-security-policy", "connect-src 'self'")], Html("<main>ready</main>")) }))
                .route("/frames", get(move || { let frame = frame.clone(); async { Html(frame) } }))
                .route("/worker.js", get(move || { let worker = worker.clone(); async { ([("content-type", "text/javascript")], worker) } }))
                .route("/sw.js", get(|| async {
                    ([("content-type", "text/javascript")],
                     "oninstall=e=>e.waitUntil(skipWaiting()); onactivate=e=>e.waitUntil(clients.claim()); onfetch=()=>{};")
                }));
            let origin_task = tokio::spawn(async move { axum::serve(origin, origin_app).await.unwrap() });
            Ok::<_, anyhow::Error>((origin_url, target_url, vec![target_task, origin_task]))
        })?;
        Ok(Self {
            _runtime: runtime,
            tasks,
            origin,
            target,
            requests,
        })
    }

    fn eval(&self, page: &str, expression: &str, disabled: bool) -> Result<Value> {
        self.eval_with_profile(page, expression, disabled, None)
    }

    fn eval_with_profile(
        &self,
        page: &str,
        expression: &str,
        disabled: bool,
        profile: Option<&str>,
    ) -> Result<Value> {
        let page = format!("{}{page}", self.origin);
        let expression = format!("{READ}; {expression}");
        let mut args = vec![
            "moli",
            "fetch",
            "--http-no-proxy",
            "*",
            "--log-level",
            "error",
            "--wait",
            "load",
            "--eval",
            &expression,
        ];
        if disabled {
            args.push("--disable-web-security");
        }
        if let Some(profile) = profile {
            args.extend(["--profile-dir", profile]);
        }
        args.push(&page);
        let output = run_moli(args)?;
        assert!(output.status.success(), "{}", clean_output(&output.stderr));
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    fn read(&self, path: &str, api: &str, disabled: bool) -> Result<Value> {
        self.eval(
            "/",
            &format!(
                "read({}, {}).catch(error => ({{error:error.message, name:error.name}}))",
                json!(format!("{}{path}", self.target)),
                json!(api)
            ),
            disabled,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

fn assert_readable(result: &Value) {
    assert_eq!(result["body"], "payload", "{result}");
    assert_eq!(result["header"], "visible", "{result}");
    assert!(
        result["cookie"].is_null(),
        "forbidden response headers stay hidden: {result}"
    );
}

#[test]
fn disabled_web_security_allows_fetch_and_both_xhr_paths_without_changing_defaults() -> Result<()> {
    let fixture = Fixture::new()?;
    for api in ["fetch", "xhr", "sync-xhr"] {
        for disabled in [false, true, false] {
            fixture.requests.lock().clear();
            let result = fixture.read("/target", api, disabled)?;
            if disabled {
                assert_readable(&result);
                if api == "fetch" {
                    assert_eq!(result["mode"], "cors");
                }
            } else {
                assert!(result["error"].is_string(), "{api}: {result}");
                if api == "fetch" {
                    assert_eq!(result["name"], "TypeError");
                    assert!(
                        result["error"]
                            .as_str()
                            .unwrap()
                            .contains("CORS check failed:")
                    );
                }
            }
            let requests = fixture.requests.lock();
            assert_eq!(requests.len(), 1, "the live server must handle the request");
            assert_eq!(requests[0].method, Method::GET);
        }
    }
    Ok(())
}

#[test]
fn disabled_web_security_skips_preflight_and_preserves_request_data() -> Result<()> {
    let fixture = Fixture::new()?;
    let target = json!(format!("{}/target", fixture.target));
    for api in ["fetch", "xhr", "sync-xhr"] {
        for disabled in [false, true] {
            fixture.requests.lock().clear();
            let result = fixture.eval("/", &format!(
                "read({target}, {}, {{method:'PUT', headers:{{'X-Test':'unchanged'}}, body:'request-body', credentials:'omit'}}).catch(error=>({{error:error.message}}))", json!(api)
            ), disabled)?;
            let requests = fixture.requests.lock();
            assert_eq!(requests.len(), 1, "{api}: {requests:?}");
            if disabled {
                assert_readable(&result);
                assert_eq!(requests[0].method, Method::PUT);
                assert_eq!(requests[0].headers["x-test"], "unchanged");
                assert_eq!(requests[0].body.as_ref(), b"request-body");
                assert!(!requests[0].headers.contains_key("cookie"));
                assert!(!requests[0].headers.contains_key("origin"));
            } else {
                assert!(result["error"].is_string(), "{result}");
                assert_eq!(requests[0].method, Method::OPTIONS);
            }
        }
    }
    Ok(())
}

#[test]
fn disabled_web_security_applies_to_redirects() -> Result<()> {
    let fixture = Fixture::new()?;
    for api in ["fetch", "xhr", "sync-xhr"] {
        let rejected = fixture.read("/redirect", api, false)?;
        assert!(rejected["error"].is_string(), "{rejected}");
        let result = fixture.read("/redirect", api, true)?;
        assert_readable(&result);
        if api == "fetch" {
            assert_eq!(result["redirected"], true);
        }
    }
    Ok(())
}

#[test]
fn workers_inherit_instance_security_policy_for_fetch_and_xhr() -> Result<()> {
    let fixture = Fixture::new()?;
    for api in ["fetch", "xhr", "sync-xhr"] {
        for disabled in [false, true] {
            let result = fixture.eval("/", &format!(r#"
                new Promise((resolve, reject) => {{
                  const worker = new Worker('/worker.js');
                  const timer = setTimeout(() => {{ worker.terminate(); reject(Error('worker timeout')); }}, 5000);
                  worker.onmessage = e => {{ clearTimeout(timer); worker.terminate(); resolve(e.data); }};
                  worker.postMessage({{url:{}, api:{}}});
                }})
            "#, json!(format!("{}/target", fixture.target)), json!(api)), disabled)?;
            if disabled {
                assert_readable(&result);
            } else {
                assert!(result["error"].is_string(), "{result}");
            }
        }
    }
    Ok(())
}

#[test]
fn disabled_web_security_exposes_basic_responses_without_rewriting_request_mode() -> Result<()> {
    let fixture = Fixture::new()?;
    let target = json!(format!("{}/target", fixture.target));
    for mode in ["cors", "no-cors", "same-origin"] {
        let result = fixture.eval("/", &format!(
            "(async()=>{{const request=new Request({target}, {{mode:{mode:?}}}); const r=await fetch(request); return {{mode:request.mode,type:r.type,text:await r.text()}};}})()"
        ), true)?;
        assert_eq!(result, json!({"mode":mode,"type":"basic","text":"payload"}));
    }
    Ok(())
}

#[test]
fn disabled_web_security_preserves_csp_before_network_dispatch() -> Result<()> {
    let fixture = Fixture::new()?;
    let target = json!(format!("{}/target", fixture.target));
    for disabled in [false, true] {
        fixture.requests.lock().clear();
        assert_eq!(
            fixture.eval(
                "/csp",
                &format!("fetch({target}).then(()=>false,()=>true)"),
                disabled
            )?,
            true
        );
        assert!(fixture.requests.lock().is_empty());
    }
    Ok(())
}

#[test]
fn disabled_web_security_grants_local_window_access_without_changing_origin() -> Result<()> {
    let fixture = Fixture::new()?;
    for disabled in [false, true, false] {
        let result = fixture.eval("/frames", r#"(()=>{
            const w=document.querySelector('iframe').contentWindow;
            try { return {origin:location.origin, targetOrigin:w.origin, text:w.document.body.textContent}; }
            catch(e) { return {origin:location.origin, error:e.name}; }
        })()"#, disabled)?;
        let expected = if disabled {
            json!({"origin":fixture.origin,"targetOrigin":fixture.target,"text":"cross-origin document"})
        } else {
            json!({"origin":fixture.origin,"error":"SecurityError"})
        };
        assert_eq!(result, expected);
    }
    Ok(())
}

#[test]
fn service_worker_network_fallback_inherits_cors_configuration() -> Result<()> {
    let fixture = Fixture::new()?;
    for disabled in [false, true] {
        let result = fixture.eval("/", &format!(r#"
            (async () => {{
              const registration = await navigator.serviceWorker.register('/sw.js');
              await navigator.serviceWorker.ready;
              if (!navigator.serviceWorker.controller) await new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {{once:true}}));
              try {{ return await read({}, 'fetch'); }}
              catch(error) {{ return {{error:error.message}}; }}
              finally {{ await registration.unregister(); }}
            }})()
        "#, json!(format!("{}/target", fixture.target))), disabled)?;
        if disabled {
            assert_readable(&result);
        } else {
            assert!(result["error"].is_string(), "{result}");
        }
    }
    Ok(())
}

#[test]
fn reused_profile_uses_each_launch_web_security_policy() -> Result<()> {
    let fixture = Fixture::new()?;
    let profile_dir = super::unique_temp_dir("web-security-reused-profile")?;
    let profile = profile_dir.to_str().unwrap();
    let target = json!(format!("{}/target", fixture.target));
    for disabled in [true, false, true] {
        let result = fixture.eval_with_profile(
            "/",
            &format!("(async()=>({{result:await fetch({target}).then(r=>r.text(),e=>e.name)}}))()"),
            disabled,
            Some(profile),
        )?;
        assert_eq!(
            result["result"],
            if disabled { "payload" } else { "TypeError" }
        );
    }
    Ok(())
}
