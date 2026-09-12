use super::*;
use crate::browser::{NetworkOwner, NetworkRequestState, WorkerHandle};
use crate::page::{
    RendererNetworkOutputItem, ScriptNetworkOutputItem, SubresourceBodyFinishedResult,
};

#[derive(Clone, Copy, Debug)]
enum WorkerKind {
    Dedicated,
    Nested,
    Shared,
    Service,
}

#[derive(Clone, Copy, Debug)]
enum RequestKind {
    Fetch,
    NoCorsFetch,
    ManualFetch,
    PreflightFetch,
    Xhr,
    SyncXhr,
    PreflightXhr,
    PreflightSyncXhr,
    ImportScript,
    StaticModule,
    DynamicModule,
    CspReport,
    ModuleCspReport,
}

impl RequestKind {
    fn needs_preflight(self) -> bool {
        matches!(
            self,
            Self::PreflightFetch | Self::PreflightXhr | Self::PreflightSyncXhr
        )
    }

    fn is_report(self) -> bool {
        matches!(self, Self::CspReport | Self::ModuleCspReport)
    }

    fn is_script(self) -> bool {
        matches!(
            self,
            Self::ImportScript | Self::StaticModule | Self::DynamicModule
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum Finish {
    Complete,
    FollowedRedirect,
    UnfollowedRedirect,
    PartialFailure,
    DetachedKeepalive,
    DetachedBeforeHeadersFailure,
    RetiredCancellation,
}

impl Finish {
    fn keepalive(self) -> bool {
        matches!(
            self,
            Self::DetachedKeepalive | Self::DetachedBeforeHeadersFailure
        )
    }

    fn retires_before_headers(self) -> bool {
        self.keepalive() || matches!(self, Self::RetiredCancellation)
    }
}

#[tokio::test]
async fn native_worker_stages_dedicated_fetch() {
    worker_network_stages(WorkerKind::Dedicated, Finish::Complete).await;
}

#[tokio::test]
async fn native_worker_stages_nested_fetch() {
    worker_network_stages(WorkerKind::Nested, Finish::Complete).await;
}

#[tokio::test]
async fn native_worker_stages_shared_fetch() {
    worker_network_stages(WorkerKind::Shared, Finish::Complete).await;
}

#[tokio::test]
async fn native_worker_stages_service_fetch() {
    worker_network_stages(WorkerKind::Service, Finish::Complete).await;
}

#[tokio::test]
async fn native_worker_stages_partial_failure_retains_received_body() {
    worker_network_stages(WorkerKind::Dedicated, Finish::PartialFailure).await;
}

#[tokio::test]
async fn native_worker_stages_detached_keepalive_keeps_original_request_and_releases_it() {
    worker_network_stages(WorkerKind::Dedicated, Finish::DetachedKeepalive).await;
}

#[tokio::test]
async fn native_worker_stages_nested_detached_keepalive() {
    worker_network_stages(WorkerKind::Nested, Finish::DetachedKeepalive).await;
}

#[tokio::test]
async fn native_worker_stages_shared_detached_keepalive() {
    worker_network_stages(WorkerKind::Shared, Finish::DetachedKeepalive).await;
}

#[tokio::test]
async fn native_worker_stages_service_detached_keepalive() {
    worker_network_stages(WorkerKind::Service, Finish::DetachedKeepalive).await;
}

#[tokio::test]
async fn native_worker_stages_detached_failure_before_headers() {
    worker_network_stages(WorkerKind::Dedicated, Finish::DetachedBeforeHeadersFailure).await;
}

#[tokio::test]
async fn native_worker_stages_dedicated_xhr() {
    worker_network_stages_with_request(WorkerKind::Dedicated, Finish::Complete, RequestKind::Xhr)
        .await;
}

#[tokio::test]
async fn native_worker_stages_nested_xhr() {
    worker_network_stages_with_request(WorkerKind::Nested, Finish::Complete, RequestKind::Xhr)
        .await;
}

#[tokio::test]
async fn native_worker_stages_shared_xhr() {
    worker_network_stages_with_request(WorkerKind::Shared, Finish::Complete, RequestKind::Xhr)
        .await;
}

#[tokio::test]
async fn native_worker_stages_dedicated_sync_xhr() {
    worker_network_stages_with_request(
        WorkerKind::Dedicated,
        Finish::Complete,
        RequestKind::SyncXhr,
    )
    .await;
}

#[tokio::test]
async fn native_worker_stages_shared_sync_xhr() {
    worker_network_stages_with_request(WorkerKind::Shared, Finish::Complete, RequestKind::SyncXhr)
        .await;
}

#[tokio::test]
async fn native_worker_stages_xhr_partial_failure_retains_received_body() {
    worker_network_stages_with_request(
        WorkerKind::Dedicated,
        Finish::PartialFailure,
        RequestKind::Xhr,
    )
    .await;
}

#[tokio::test]
async fn native_worker_stages_sync_xhr_partial_failure_retains_received_body() {
    worker_network_stages_with_request(
        WorkerKind::Dedicated,
        Finish::PartialFailure,
        RequestKind::SyncXhr,
    )
    .await;
}

#[tokio::test]
async fn native_worker_stages_dedicated_xhr_retirement_cancels_request() {
    worker_network_stages_with_request(
        WorkerKind::Dedicated,
        Finish::RetiredCancellation,
        RequestKind::Xhr,
    )
    .await;
}

#[tokio::test]
async fn native_worker_stages_nested_xhr_retirement_cancels_request() {
    worker_network_stages_with_request(
        WorkerKind::Nested,
        Finish::RetiredCancellation,
        RequestKind::Xhr,
    )
    .await;
}

#[tokio::test]
async fn native_worker_stages_shared_xhr_retirement_cancels_request() {
    worker_network_stages_with_request(
        WorkerKind::Shared,
        Finish::RetiredCancellation,
        RequestKind::Xhr,
    )
    .await;
}

#[tokio::test]
async fn native_worker_stages_dedicated_sync_xhr_retirement_cancels_request() {
    worker_network_stages_with_request(
        WorkerKind::Dedicated,
        Finish::RetiredCancellation,
        RequestKind::SyncXhr,
    )
    .await;
}

#[tokio::test]
async fn native_worker_stages_nested_sync_xhr_retirement_cancels_request() {
    worker_network_stages_with_request(
        WorkerKind::Nested,
        Finish::RetiredCancellation,
        RequestKind::SyncXhr,
    )
    .await;
}

#[tokio::test]
async fn native_worker_stages_shared_sync_xhr_retirement_cancels_request() {
    worker_network_stages_with_request(
        WorkerKind::Shared,
        Finish::RetiredCancellation,
        RequestKind::SyncXhr,
    )
    .await;
}

macro_rules! worker_stage_tests {
    ($($name:ident: $worker:ident, $request:ident, $finish:ident;)*) => {
        $(
            #[tokio::test]
            async fn $name() {
                worker_network_stages_with_request(
                    WorkerKind::$worker, Finish::$finish, RequestKind::$request,
                ).await;
            }
        )*
    };
}

worker_stage_tests! {
    native_worker_filtered_stages_no_cors: Dedicated, NoCorsFetch, Complete;
    native_worker_filtered_stages_shared_no_cors: Shared, NoCorsFetch, Complete;
    native_worker_filtered_stages_service_no_cors: Service, NoCorsFetch, Complete;
    native_worker_filtered_stages_no_cors_partial: Dedicated, NoCorsFetch, PartialFailure;
    native_worker_filtered_stages_no_cors_detached: Dedicated, NoCorsFetch, DetachedKeepalive;
    native_worker_filtered_stages_manual: Dedicated, ManualFetch, UnfollowedRedirect;
    native_worker_filtered_stages_manual_partial: Dedicated, ManualFetch, PartialFailure;
    native_worker_filtered_stages_preflight_fetch: Dedicated, PreflightFetch, Complete;
    native_worker_filtered_stages_preflight_redirect: Dedicated, PreflightFetch, FollowedRedirect;
    native_worker_filtered_stages_preflight_manual: Dedicated, PreflightFetch, UnfollowedRedirect;
    native_worker_filtered_stages_preflight_detached: Dedicated, PreflightFetch, DetachedKeepalive;
    native_worker_filtered_stages_shared_preflight_fetch: Shared, PreflightFetch, Complete;
    native_worker_filtered_stages_preflight_fetch_partial: Dedicated, PreflightFetch, PartialFailure;
    native_worker_filtered_stages_preflight_xhr: Dedicated, PreflightXhr, Complete;
    native_worker_filtered_stages_preflight_xhr_redirect: Dedicated, PreflightXhr, FollowedRedirect;
    native_worker_filtered_stages_preflight_sync_xhr: Dedicated, PreflightSyncXhr, Complete;
    native_worker_filtered_stages_preflight_xhr_partial: Dedicated, PreflightXhr, PartialFailure;
    native_worker_filtered_stages_preflight_sync_xhr_retired: Dedicated, PreflightSyncXhr, RetiredCancellation;
    native_worker_csp_stages_redirect_body: Dedicated, CspReport, UnfollowedRedirect;
    native_worker_csp_stages_dynamic_module: Dedicated, ModuleCspReport, Complete;
    native_worker_csp_stages_shared_dynamic_module: Shared, ModuleCspReport, Complete;
    native_worker_csp_stages_dynamic_module_detached: Dedicated, ModuleCspReport, DetachedKeepalive;
    native_worker_csp_stages_dedicated: Dedicated, CspReport, Complete;
    native_worker_csp_stages_nested: Nested, CspReport, Complete;
    native_worker_csp_stages_shared: Shared, CspReport, Complete;
    native_worker_csp_stages_service: Service, CspReport, Complete;
    native_worker_csp_stages_partial_failure: Dedicated, CspReport, PartialFailure;
    native_worker_csp_stages_dedicated_detached: Dedicated, CspReport, DetachedKeepalive;
    native_worker_csp_stages_nested_detached: Nested, CspReport, DetachedKeepalive;
    native_worker_csp_stages_shared_detached: Shared, CspReport, DetachedKeepalive;
    native_worker_csp_stages_service_detached: Service, CspReport, DetachedKeepalive;
    native_worker_csp_stages_detached_failure: Dedicated, CspReport, DetachedBeforeHeadersFailure;
    native_worker_script_stages_dedicated_import: Dedicated, ImportScript, Complete;
    native_worker_script_stages_nested_import: Nested, ImportScript, Complete;
    native_worker_script_stages_shared_import: Shared, ImportScript, Complete;
    native_worker_script_stages_service_import: Service, ImportScript, Complete;
    native_worker_script_stages_dedicated_static_module: Dedicated, StaticModule, Complete;
    native_worker_script_stages_nested_static_module: Nested, StaticModule, Complete;
    native_worker_script_stages_shared_static_module: Shared, StaticModule, Complete;
    native_worker_script_stages_service_static_module: Service, StaticModule, Complete;
    native_worker_script_stages_dedicated_dynamic_module: Dedicated, DynamicModule, Complete;
    native_worker_script_stages_nested_dynamic_module: Nested, DynamicModule, Complete;
    native_worker_script_stages_shared_dynamic_module: Shared, DynamicModule, Complete;
    native_worker_script_stages_import_partial_failure: Dedicated, ImportScript, PartialFailure;
    native_worker_script_stages_static_module_partial_failure: Dedicated, StaticModule, PartialFailure;
    native_worker_script_stages_dynamic_module_partial_failure: Dedicated, DynamicModule, PartialFailure;
    native_worker_script_stages_dedicated_import_retirement: Dedicated, ImportScript, RetiredCancellation;
    native_worker_script_stages_nested_import_retirement: Nested, ImportScript, RetiredCancellation;
    native_worker_script_stages_shared_import_retirement: Shared, ImportScript, RetiredCancellation;
    native_worker_script_stages_service_import_retirement: Service, ImportScript, RetiredCancellation;
    native_worker_script_stages_dedicated_static_module_retirement: Dedicated, StaticModule, RetiredCancellation;
    native_worker_script_stages_nested_static_module_retirement: Nested, StaticModule, RetiredCancellation;
    native_worker_script_stages_shared_static_module_retirement: Shared, StaticModule, RetiredCancellation;
    native_worker_script_stages_service_static_module_retirement: Service, StaticModule, RetiredCancellation;
}

async fn worker_network_stages(kind: WorkerKind, finish: Finish) {
    worker_network_stages_with_request(kind, finish, RequestKind::Fetch).await;
}

async fn worker_network_stages_with_request(
    kind: WorkerKind,
    finish: Finish,
    request_kind: RequestKind,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let cross_origin_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let request_path = if matches!(finish, Finish::FollowedRedirect) {
        "/redirect"
    } else {
        "/probe"
    };
    let url = if request_kind.needs_preflight() || matches!(request_kind, RequestKind::NoCorsFetch)
    {
        format!(
            "http://{}{request_path}",
            cross_origin_listener.local_addr().unwrap()
        )
    } else {
        format!("{origin}{request_path}")
    };
    let request_url = url.clone();
    let page_origin = origin.clone();
    let response_body = if request_kind.is_script() {
        "//ok"
    } else {
        "body"
    };
    let (requested, request_arrived) = oneshot::channel();
    let (headers, release_headers) = oneshot::channel();
    let (chunk, release_chunk) = oneshot::channel();
    let (tail, release_tail) = oneshot::channel();
    let server = tokio::spawn(async move {
        let request_script = match request_kind {
            RequestKind::Fetch => format!(
                "fetch('/probe',{{keepalive:{}}}).then(r=>r.text()).catch(()=>{{}})",
                finish.keepalive()
            ),
            RequestKind::NoCorsFetch => format!(
                "fetch({request_url:?},{{mode:'no-cors',keepalive:{}}}).then(r=>r.text()).catch(()=>{{}})",
                finish.keepalive()
            ),
            RequestKind::ManualFetch => format!(
                "fetch({request_url:?},{{redirect:'manual'}}).then(r=>r.text()).catch(()=>{{}})"
            ),
            RequestKind::PreflightFetch => format!(
                "fetch({request_url:?},{{method:'PUT',headers:{{'x-native-probe':'yes'}},keepalive:{},redirect:'{}'}}).then(r=>r.text()).catch(()=>{{}})",
                finish.keepalive(),
                if matches!(finish, Finish::UnfollowedRedirect) {
                    "manual"
                } else {
                    "follow"
                }
            ),
            RequestKind::Xhr | RequestKind::SyncXhr => format!(
                "try{{const xhr=new XMLHttpRequest();xhr.open('GET','/probe',{});xhr.send();}}catch(_){{}}",
                matches!(request_kind, RequestKind::Xhr)
            ),
            RequestKind::PreflightXhr | RequestKind::PreflightSyncXhr => format!(
                "try{{const xhr=new XMLHttpRequest();xhr.open('PUT',{request_url:?},{});xhr.setRequestHeader('x-native-probe','yes');xhr.send();}}catch(_){{}}",
                matches!(request_kind, RequestKind::PreflightXhr)
            ),
            RequestKind::ImportScript => "try{importScripts('/probe')}catch(_){}".into(),
            RequestKind::StaticModule => "import '/probe';".into(),
            RequestKind::DynamicModule => "import('/probe').catch(()=>{})".into(),
            RequestKind::CspReport => "fetch('/blocked').catch(()=>{})".into(),
            RequestKind::ModuleCspReport => "import('/blocked').catch(()=>{})".into(),
        };
        let options = if matches!(request_kind, RequestKind::StaticModule) {
            "{type:'module'}"
        } else {
            "{}"
        };
        let worker_script = match kind {
            WorkerKind::Dedicated => request_script.clone(),
            WorkerKind::Nested => format!("globalThis.child = new Worker('/nested.js',{options})"),
            WorkerKind::Shared if request_kind.is_script() => {
                format!("{request_script};onconnect=()=>{{}}")
            }
            WorkerKind::Shared => format!("onconnect=()=>{{{request_script}}}"),
            WorkerKind::Service
                if matches!(
                    request_kind,
                    RequestKind::ImportScript | RequestKind::StaticModule
                ) =>
            {
                request_script.clone()
            }
            WorkerKind::Service => {
                assert!(matches!(
                    request_kind,
                    RequestKind::Fetch | RequestKind::NoCorsFetch | RequestKind::CspReport
                ));
                format!("addEventListener('install',event=>event.waitUntil({request_script}))")
            }
        };
        let bootstrap = match kind {
            WorkerKind::Dedicated => {
                format!("globalThis.worker = new Worker('/worker.js',{options})")
            }
            WorkerKind::Nested => "globalThis.worker = new Worker('/worker.js')".into(),
            WorkerKind::Shared => {
                format!(
                    "globalThis.worker = new SharedWorker('/worker.js',{options});worker.port.start()"
                )
            }
            WorkerKind::Service => {
                format!("navigator.serviceWorker.register('/worker.js',{options})")
            }
        };
        let html = format!("<!doctype html><script>{bootstrap}</script>");
        let mut preflight_count = 0;
        loop {
            let (mut stream, _) = tokio::select! {
                accepted = listener.accept() => accepted,
                accepted = cross_origin_listener.accept() => accepted,
            }
            .unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            let path = request.split_whitespace().nth(1).unwrap();
            if request.starts_with("OPTIONS ") {
                assert!(request_kind.needs_preflight());
                assert!(path == "/probe" || path == "/redirect");
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("access-control-request-method: put\r\n")
                );
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("access-control-request-headers: x-native-probe\r\n")
                );
                preflight_count += 1;
                stream.write_all(format!("HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: {page_origin}\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: x-native-probe\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                continue;
            }
            if path == "/redirect" {
                assert!(matches!(finish, Finish::FollowedRedirect));
                assert_eq!(preflight_count, 1);
                assert!(request.starts_with("PUT /redirect HTTP/1.1"));
                stream.write_all(format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: /probe\r\nAccess-Control-Allow-Origin: {page_origin}\r\nContent-Length: 8\r\nConnection: close\r\n\r\nredirect").as_bytes()).await.unwrap();
                continue;
            }
            if path == "/probe" {
                assert_eq!(
                    preflight_count,
                    usize::from(request_kind.needs_preflight())
                        + usize::from(matches!(finish, Finish::FollowedRedirect))
                );
                if request_kind.needs_preflight() {
                    assert!(request.starts_with("PUT /probe HTTP/1.1"));
                    assert!(
                        request
                            .to_ascii_lowercase()
                            .contains("x-native-probe: yes\r\n")
                    );
                }
                if request_kind.is_report() {
                    assert!(request.starts_with("POST /probe HTTP/1.1"));
                    let length = request
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .expect("report body length");
                    let mut body = vec![0; length];
                    stream.read_exact(&mut body).await.unwrap();
                    let report: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    assert_eq!(
                        report["csp-report"]["effective-directive"],
                        if matches!(request_kind, RequestKind::ModuleCspReport) {
                            "script-src-elem"
                        } else {
                            "connect-src"
                        }
                    );
                    assert!(
                        report["csp-report"]["blocked-uri"]
                            .as_str()
                            .unwrap()
                            .ends_with("/blocked")
                    );
                }
                requested.send(()).unwrap();
                release_headers.await.unwrap();
                if matches!(
                    finish,
                    Finish::DetachedBeforeHeadersFailure | Finish::RetiredCancellation
                ) {
                    break;
                }
                let mime = if request_kind.is_script() {
                    "text/javascript"
                } else if matches!(request_kind, RequestKind::NoCorsFetch) {
                    "image/png"
                } else {
                    "text/plain"
                };
                let status = if matches!(finish, Finish::UnfollowedRedirect) {
                    "302 Found\r\nLocation: /must-not-follow"
                } else {
                    "200 OK"
                };
                stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nAccess-Control-Allow-Origin: {page_origin}\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                release_chunk.await.unwrap();
                stream
                    .write_all(&response_body.as_bytes()[..2])
                    .await
                    .unwrap();
                release_tail.await.unwrap();
                if !matches!(finish, Finish::PartialFailure) {
                    stream
                        .write_all(&response_body.as_bytes()[2..])
                        .await
                        .unwrap();
                }
                break;
            }
            let (content_type, body) = match path {
                "/" => ("text/html", html.as_str()),
                "/worker.js" => ("text/javascript", worker_script.as_str()),
                "/nested.js" => ("text/javascript", request_script.as_str()),
                other => panic!("unexpected Worker fixture request: {other}"),
            };
            let csp = if request_kind.is_report() && path != "/" {
                if matches!(request_kind, RequestKind::ModuleCspReport) {
                    "Content-Security-Policy: script-src 'none'; report-uri /probe\r\n"
                } else {
                    "Content-Security-Policy: connect-src 'none'; report-uri /probe\r\n"
                }
            } else {
                ""
            };
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n{csp}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    navigate(&context, contents, &format!("{origin}/")).await;
    tokio::time::timeout(std::time::Duration::from_secs(5), request_arrived)
        .await
        .expect("the real Worker must dispatch its request before any response is released")
        .unwrap();
    let (owner, source, handle, mut sequence) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        async {
            loop {
                let event = events.recv().await.unwrap();
                if let BrowserEvent::NetworkRequestStarted(occurrence) = event.event
                    && let NetworkOwner::Worker(owner) = occurrence.owner
                    && let RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item
                    && let ScriptNetworkOutputItem::SubresourceRequestStarted(request) =
                        item.as_ref()
                    && request.url().as_str() == url
                {
                    assert_eq!(
                        request.keepalive(),
                        finish.keepalive() || request_kind.is_report()
                    );
                    assert_eq!(
                        request.resource_type(),
                        match request_kind {
                            RequestKind::Fetch
                            | RequestKind::NoCorsFetch
                            | RequestKind::ManualFetch
                            | RequestKind::PreflightFetch =>
                                crate::page::SubresourceResourceType::Fetch,
                            RequestKind::CspReport | RequestKind::ModuleCspReport =>
                                crate::page::SubresourceResourceType::CspReport,
                            RequestKind::Xhr
                            | RequestKind::SyncXhr
                            | RequestKind::PreflightXhr
                            | RequestKind::PreflightSyncXhr =>
                                crate::page::SubresourceResourceType::Xhr,
                            RequestKind::ImportScript
                            | RequestKind::StaticModule
                            | RequestKind::DynamicModule =>
                                crate::page::SubresourceResourceType::Script,
                        }
                    );
                    break (
                        owner,
                        occurrence.renderer.source.clone(),
                        request.handle(),
                        event.sequence,
                    );
                }
            }
        },
    )
    .await
    .expect(
        "native Worker Started must precede the held response headers without a DevTools consumer",
    );
    assert!(matches!(
        (kind, owner),
        (
            WorkerKind::Dedicated | WorkerKind::Nested,
            WorkerHandle::Dedicated { .. }
        ) | (WorkerKind::Shared, WorkerHandle::Shared { .. })
            | (WorkerKind::Service, WorkerHandle::Service { .. })
    ));
    assert!(browser.subscribe().unwrap().0.network_requests.iter().any(|request|
        request.owner == NetworkOwner::Worker(owner) && request.renderer_source == source
        && matches!(&request.state, NetworkRequestState::Started(start) if start.handle() == handle)));
    let mut buffered = std::collections::VecDeque::new();
    if finish.retires_before_headers() {
        if let crate::page::RendererNetworkSource::Worker(
            crate::page::RendererWorkerIdentity::Service { run, .. },
        ) = &source
        {
            assert!(browser.subscribe().unwrap().0.workers.iter().any(|worker|
                worker.handle() == owner && matches!(worker, crate::browser::WorkerSnapshot::Service { worker, .. } if worker.execution.active_run() == Some(run))));
        }
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            context.close_web_contents(contents).unwrap().close_async(),
        )
        .await
        .expect("Worker retirement must finish with the response headers held");
        if let WorkerHandle::Service { version, .. } = owner {
            context
                .execute_service_worker_command(crate::browser::ServiceWorkerCommand::StopVersion {
                    version_id: version,
                })
                .unwrap();
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let event = events.recv().await.unwrap();
                let retired = match &event.event {
                    BrowserEvent::WorkerDestroyed(closed) => *closed == owner,
                    BrowserEvent::WorkerUpdated(worker) if worker.handle() == owner => matches!(worker,
                        crate::browser::WorkerSnapshot::Service { worker, .. } if worker.execution == crate::browser::ServiceWorkerExecution::Stopped),
                    _ => false,
                };
                if retired {
                    assert!(event.sequence > sequence);
                    break;
                }
                // Cancellation can finish before the WorkerDestroyed fact.
                // Preserve the real FIFO instead of discarding that completion.
                buffered.push_back(event);
            }
        })
        .await
        .expect("the original Worker must retire while its response is held");
        assert!(
            browser
                .subscribe()
                .unwrap()
                .0
                .workers
                .iter()
                .all(|worker| worker.handle() != owner || matches!(worker,
                    crate::browser::WorkerSnapshot::Service { worker, .. } if worker.execution == crate::browser::ServiceWorkerExecution::Stopped))
        );
    }
    let mut headers = Some(headers);
    if !matches!(finish, Finish::RetiredCancellation) {
        headers.take().unwrap().send(()).unwrap();
    }
    let stages = if matches!(
        finish,
        Finish::DetachedBeforeHeadersFailure | Finish::RetiredCancellation
    ) {
        vec![(2, None)]
    } else {
        vec![(0, Some(chunk)), (1, Some(tail)), (2, None)]
    };
    for (stage, release) in stages {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let event = match buffered.pop_front() {
                    Some(event) => event,
                    None => events.recv().await.unwrap(),
                };
                let occurrence = match &event.event {
                    BrowserEvent::NetworkActivity(occurrence)
                    | BrowserEvent::NetworkRequestCompleted(occurrence)
                        if occurrence.owner == NetworkOwner::Worker(owner) => occurrence,
                    _ => continue,
                };
                assert_eq!(occurrence.renderer.source, source);
                let RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item else { continue };
                let matches = match (stage, item.as_ref()) {
                    (0, ScriptNetworkOutputItem::SubresourceResponseStarted(response)) => response.handle() == handle,
                    (1, ScriptNetworkOutputItem::SubresourceDataReceived(data)) => data.handle() == handle,
                    (2, ScriptNetworkOutputItem::SubresourceBodyFinished(body)) => body.handle() == handle,
                    _ => false,
                };
                if matches { break event; }
            }
        }).await.unwrap_or_else(|_| panic!("Worker {kind:?} {finish:?} must publish stage {stage} before the next transport gate opens"));
        assert!(event.sequence > sequence);
        sequence = event.sequence;
        match stage {
            0 => {
                assert!(browser.subscribe().unwrap().0.network_requests.iter().any(|request|
                    request.owner == NetworkOwner::Worker(owner) && request.renderer_source == source
                    && matches!(&request.state, NetworkRequestState::Responding { response, .. } if response.handle() == handle)));
            }
            1 => {}
            2 => {
                let BrowserEvent::NetworkRequestCompleted(occurrence) = event.event else {
                    panic!("the final body is a native completion, not generic activity");
                };
                let RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item else {
                    unreachable!()
                };
                let ScriptNetworkOutputItem::SubresourceBodyFinished(body) = item.as_ref() else {
                    unreachable!()
                };
                match (finish, body.result()) {
                    (
                        Finish::DetachedBeforeHeadersFailure | Finish::RetiredCancellation,
                        SubresourceBodyFinishedResult::Failed(error),
                    ) => assert!(!error.is_empty()),
                    (
                        Finish::Complete
                        | Finish::FollowedRedirect
                        | Finish::UnfollowedRedirect
                        | Finish::DetachedKeepalive,
                        SubresourceBodyFinishedResult::Ready(body),
                    ) => assert_eq!(body.clone_body_bytes(), response_body.as_bytes()),
                    (
                        Finish::PartialFailure,
                        SubresourceBodyFinishedResult::FailedWithPartialBody {
                            error_text,
                            partial_body,
                        },
                    ) => {
                        assert!(!error_text.is_empty());
                        assert_eq!(
                            partial_body.clone_body_bytes(),
                            &response_body.as_bytes()[..2]
                        );
                    }
                    other => {
                        panic!("native terminal must retain the actual transport result: {other:?}")
                    }
                }
            }
            _ => unreachable!(),
        }
        if let Some(release) = release {
            release.send(()).unwrap();
        }
    }
    // Only let the server close after observing native cancellation. Otherwise
    // a fixture-induced disconnect could falsely prove retirement cancellation.
    if let Some(headers) = headers {
        headers.send(()).unwrap();
    }
    server.await.unwrap();
    if let WorkerHandle::Service { version, .. } = owner
        && !finish.retires_before_headers()
    {
        context
            .execute_service_worker_command(crate::browser::ServiceWorkerCommand::StopVersion {
                version_id: version,
            })
            .unwrap();
    }
    if !finish.retires_before_headers() {
        context
            .close_web_contents(contents)
            .unwrap()
            .close_async()
            .await;
    }
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if browser
                .subscribe()
                .unwrap()
                .0
                .network_requests
                .iter()
                .all(|request| request.renderer_source != source)
            {
                break;
            }
            events.recv().await.unwrap();
        }
    })
    .await
    .expect(
        "retired physical source and its completed keepalive tail must release native retention",
    );
    service.shutdown();
}
