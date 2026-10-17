use super::*;
use crate::network::{
    RendererResourceTaskRunner,
    context::{WorkerResourceLoader, WorkerResourceOwner},
    loads::{ResourceLoadDisposition, ResourceLoadKind, ResourceLoadLease},
};
use moli_fetch::{RequestMode, Response};
use parking_lot::Mutex;
use tokio::sync::mpsc;

const WAIT: Duration = Duration::from_secs(5);
const SCRIPT: &str = "self.cachedImportCount = (self.cachedImportCount || 0) + 1;";

struct ScriptServer {
    url: Url,
    requests: Arc<Mutex<Vec<String>>>,
    bodies: mpsc::UnboundedReceiver<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl ScriptServer {
    async fn new(cache_control: &str, delayed_body: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}/app.js", listener.local_addr().unwrap())).unwrap();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nCache-Control: {cache_control}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            SCRIPT.len(),
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let (body_tx, bodies) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let request = read_http_request_text(&mut stream).await.unwrap();
                let redirect = request.starts_with("GET /redirect.js ");
                captured.lock().push(request);
                if redirect {
                    stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: /app.js\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                    continue;
                }
                stream.write_all(response.as_bytes()).await.unwrap();
                if delayed_body {
                    let (ready_tx, ready_rx) = oneshot::channel();
                    body_tx.send(ready_tx).unwrap();
                    ready_rx.await.unwrap();
                }
                let _ = stream.write_all(SCRIPT.as_bytes()).await;
            }
        });
        Self {
            url,
            requests,
            bodies,
            task,
        }
    }

    fn count(&self) -> usize {
        self.requests.lock().len()
    }

    async fn release_body(&mut self) {
        timeout(WAIT, self.bodies.recv())
            .await
            .unwrap()
            .unwrap()
            .send(())
            .unwrap();
    }
}

impl Drop for ScriptServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn worker(loader: &ResourceRequestClient, name: &str) -> WorkerResourceLoader {
    WorkerResourceLoader::new(
        loader.fork_with_isolated_worker_network_policy(),
        WorkerResourceOwner::Dedicated { name: name.into() },
        RendererResourceTaskRunner::from_current_tokio().unwrap(),
    )
}

fn request(url: &Url) -> Request {
    Request::get_with_url(url.clone())
        .with_request_origin(url.into())
        .with_initiator_url(url)
        .with_request_mode(RequestMode::NoCors)
        .with_script_fetch_metadata(ScriptFetchRequestMetadata::default())
}

fn load(worker: &WorkerResourceLoader) -> ResourceLoadLease {
    worker
        .register_load(
            ResourceLoadKind::Script,
            ResourceLoadDisposition::Ordinary,
            None,
        )
        .unwrap()
}

fn fetch_blocking(
    worker: &WorkerResourceLoader,
    request: Request,
) -> (
    tokio::task::JoinHandle<Result<Response>>,
    oneshot::Receiver<()>,
    ResourceLoadLease,
) {
    let resource_load = load(worker);
    let cancel = resource_load.clone();
    let client = worker.request_client().clone();
    let (header_tx, header_rx) = oneshot::channel();
    let task = tokio::task::spawn_blocking(move || {
        client.fetch_cacheable_script_text_for_worker_blocking_boundary_with_load(
            request,
            resource_load,
            move || {
                let _ = header_tx.send(());
            },
        )
    });
    (task, header_rx, cancel)
}

#[tokio::test]
async fn worker_script_cache_reuses_wire_response_and_honors_no_store() {
    for (cache_control, expected_requests) in [("max-age=600", 1), ("no-store", 2)] {
        let server = ScriptServer::new(cache_control, false).await;
        let transport = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let worker = worker(&transport, "cache");
        for index in 0..2 {
            let (task, headers, _) = fetch_blocking(&worker, request(&server.url));
            timeout(WAIT, headers).await.unwrap().unwrap();
            let response = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
            assert_eq!(response.body_text(), SCRIPT);
            assert_eq!(response.from_cache, index == 1 && expected_requests == 1);
            assert_eq!(
                response.cache_state == moli_fetch::ResponseCacheState::Local,
                index == 1 && expected_requests == 1
            );
        }
        assert_eq!(server.count(), expected_requests, "{cache_control}");
    }
}

#[tokio::test]
async fn worker_script_cache_retiring_one_consumer_preserves_its_sibling() {
    for cancel_owner in [false, true] {
        let mut server = ScriptServer::new("max-age=600", true).await;
        let transport = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let first = worker(&transport, "first");
        let second = worker(&transport, "second");
        let (owner_task, owner_headers, owner_load) = fetch_blocking(&first, request(&server.url));
        timeout(WAIT, owner_headers).await.unwrap().unwrap();
        assert!(
            !owner_task.is_finished(),
            "response headers precede body completion"
        );
        let (waiter_task, waiter_headers, waiter_load) =
            fetch_blocking(&second, request(&server.url));
        timeout(WAIT, waiter_headers).await.unwrap().unwrap();
        assert!(!waiter_task.is_finished());
        let (retired, surviving, retired_load) = if cancel_owner {
            (owner_task, waiter_task, owner_load)
        } else {
            (waiter_task, owner_task, waiter_load)
        };
        retired_load.cancel();
        let error = timeout(WAIT, retired).await.unwrap().unwrap().unwrap_err();
        assert!(error.to_string().contains("consumer retired"), "{error:#}");
        assert!(
            !surviving.is_finished(),
            "retiring one consumer must not finish its sibling"
        );
        server.release_body().await;
        let response = timeout(WAIT, surviving).await.unwrap().unwrap().unwrap();
        assert_eq!(response.body_text(), SCRIPT);
        assert_eq!(server.count(), 1);
    }
}

#[tokio::test]
async fn worker_script_cache_rechecks_each_consumers_redirect_policy() {
    for pending in [false, true] {
        let mut server = ScriptServer::new("max-age=600", pending).await;
        let transport = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let worker = worker(&transport, "redirect");
        let url = server.url.join("/redirect.js").unwrap();
        let (first, headers, _) = fetch_blocking(&worker, request(&url));
        timeout(WAIT, headers).await.unwrap().unwrap();
        let first = if pending {
            Some(first)
        } else {
            assert_eq!(
                timeout(WAIT, first)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
                    .body_text(),
                SCRIPT
            );
            None
        };
        let second_request =
            request(&url).with_redirect_check(moli_fetch::RequestRedirectCheck::new(|target| {
                Err(format!("consumer CSP rejected {}", target.path()))
            }));
        let (second, second_headers, _) = fetch_blocking(&worker, second_request);
        if let Some(first) = first {
            timeout(WAIT, second_headers).await.unwrap().unwrap();
            server.release_body().await;
            assert_eq!(
                timeout(WAIT, first)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
                    .body_text(),
                SCRIPT
            );
        } else {
            assert!(
                timeout(WAIT, second_headers).await.unwrap().is_err(),
                "a rejected redirect has no final-response notification"
            );
        }
        let error = timeout(WAIT, second).await.unwrap().unwrap().unwrap_err();
        assert!(
            error.to_string().contains("consumer CSP rejected /app.js"),
            "{error:#}"
        );
        assert_eq!(
            server.count(),
            if pending { 2 } else { 3 },
            "pending consumers share a redirect; completed redirects are not retained by this cache"
        );
    }
}

#[tokio::test]
async fn worker_script_cache_reuses_initiator_paths_and_isolates_origins() {
    let server = ScriptServer::new("max-age=600", false).await;
    let transport = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
    let worker = worker(&transport, "origin");
    for (path, opaque, cached) in [
        ("/first", false, false),
        ("/second", false, true),
        ("/second", true, false),
    ] {
        let mut request = request(&server.url).with_initiator_url(&server.url.join(path).unwrap());
        if opaque {
            request = request.with_request_origin(moli_url::WebOrigin::Opaque);
        }
        let (task, headers, _) = fetch_blocking(&worker, request);
        timeout(WAIT, headers).await.unwrap().unwrap();
        let response = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
        assert_eq!(response.from_cache, cached);
        assert_eq!(response.body_text(), SCRIPT);
    }
    assert_eq!(server.count(), 2);
}
