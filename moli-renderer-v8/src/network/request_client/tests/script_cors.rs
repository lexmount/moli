use super::*;
use crate::{
    frame_owner_model::{DocumentId, FrameDocumentTaskOwner, FrameSchedulerLaneId, LocalWindowId},
    native_bridge::WindowDocumentOwner,
    network::{
        RendererResourceTaskRunner,
        context::{DocumentFetchContext, DocumentResourceLoader},
        loads::{ResourceLoadDisposition, ResourceLoadKind},
    },
};
use moli_fetch::{RequestMode, Response};
use parking_lot::Mutex;

struct ScriptServer {
    url: Url,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl ScriptServer {
    async fn new(status: u16, headers: &[(&str, &str)]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!(
            "http://{}/script.mjs",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let body = "export const value = 1;";
        let extra_headers = headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"))
            .collect::<String>();
        let response = format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: application/javascript\r\nCache-Control: max-age=3600\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}",
            body.len()
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let request = read_http_request_text(&mut stream).await.unwrap();
                captured.lock().push(request);
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().len()
    }

    fn last_origin(&self) -> Option<String> {
        self.last_header("origin")
    }

    fn last_header(&self, header: &str) -> Option<String> {
        self.requests.lock().last().and_then(|request| {
            request.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case(header)
                    .then(|| value.trim().to_owned())
            })
        })
    }
}

impl Drop for ScriptServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn document(loader: &ResourceRequestClient, url: &str, origin: &str) -> DocumentResourceLoader {
    let url = Url::parse(url).unwrap();
    DocumentResourceLoader::new(
        loader.clone(),
        RendererResourceTaskRunner::from_current_tokio().unwrap(),
        DocumentFetchContext::new(
            WindowDocumentOwner::Frame(FrameDocumentTaskOwner::new(
                FrameSchedulerLaneId(7),
                LocalWindowId(11),
                DocumentId(1),
            )),
            url.clone(),
            url,
            origin,
        ),
    )
}

fn script_request(url: &Url, mode: RequestMode, credentials: RequestCredentialsMode) -> Request {
    Request::new("GET", url.as_str(), None, Vec::new())
        .unwrap()
        // A dependency's referrer can be same-origin with the script server.
        // It must never replace the Document's origin for CORS or cookies.
        .with_initiator_url(url)
        .with_request_mode(mode)
        .with_credentials_mode(credentials)
        .with_script_fetch_metadata(ScriptFetchRequestMetadata::default())
}

async fn fetch_script(
    document: &DocumentResourceLoader,
    request: Request,
    callback: bool,
) -> Result<Response> {
    let load = document
        .register_load(
            ResourceLoadKind::Script,
            ResourceLoadDisposition::Ordinary,
            None,
        )
        .unwrap();
    let client = load.request_client();
    if callback {
        let (tx, rx) = oneshot::channel();
        client.fetch_cacheable_script_text_callback_with_load(request, load, move |result| {
            let _ = tx.send(result);
        })?;
        timeout(Duration::from_secs(5), rx).await??
    } else {
        let result = timeout(
            Duration::from_secs(5),
            client.fetch_cacheable_script_text_stream(request),
        )
        .await?;
        load.finish();
        result
    }
}

#[tokio::test]
async fn script_cors_checks_the_document_origin_and_credentials_in_both_fetch_paths() {
    for callback in [false, true] {
        for (headers, credentials, allowed) in [
            (vec![], RequestCredentialsMode::SameOrigin, false),
            (
                vec![("Access-Control-Allow-Origin", "https://wrong.test")],
                RequestCredentialsMode::SameOrigin,
                false,
            ),
            (
                vec![("Access-Control-Allow-Origin", "https://page.test")],
                RequestCredentialsMode::SameOrigin,
                true,
            ),
            (
                vec![("Access-Control-Allow-Origin", "*")],
                RequestCredentialsMode::SameOrigin,
                true,
            ),
            (
                vec![("Access-Control-Allow-Origin", "*")],
                RequestCredentialsMode::Include,
                false,
            ),
            (
                vec![("Access-Control-Allow-Origin", "https://page.test")],
                RequestCredentialsMode::Include,
                false,
            ),
            (
                vec![
                    ("Access-Control-Allow-Origin", "https://page.test"),
                    ("Access-Control-Allow-Credentials", "true"),
                ],
                RequestCredentialsMode::Include,
                true,
            ),
        ] {
            let server = ScriptServer::new(200, &headers).await;
            let transport = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let document = document(&transport, "https://page.test/page", "https://page.test");
            let result = fetch_script(
                &document,
                script_request(&server.url, RequestMode::Cors, credentials),
                callback,
            )
            .await;
            assert_eq!(
                result.is_ok(),
                allowed,
                "callback={callback}, headers={headers:?}, credentials={credentials:?}: {result:?}"
            );
            assert_eq!(server.last_origin().as_deref(), Some("https://page.test"));
            if let Err(error) = result {
                assert!(error.to_string().contains("CORS check failed"), "{error}");
            }
        }
    }
}

#[tokio::test]
async fn script_cors_preserves_same_origin_and_no_cors_loading() {
    for callback in [false, true] {
        for mode in [RequestMode::Cors, RequestMode::NoCors] {
            let server = ScriptServer::new(200, &[]).await;
            let transport = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let origin = if mode == RequestMode::Cors {
                moli_url::origin_ascii_serialization(&server.url)
            } else {
                "https://page.test".to_owned()
            };
            let document = document(&transport, server.url.as_str(), &origin);
            fetch_script(
                &document,
                script_request(&server.url, mode, RequestCredentialsMode::Include),
                callback,
            )
            .await
            .unwrap();
        }
    }
}

#[tokio::test]
async fn script_cors_cache_cannot_reuse_no_cors_or_another_documents_authorization() {
    for callback in [false, true] {
        let server = ScriptServer::new(
            200,
            &[
                ("Access-Control-Allow-Origin", "https://first.test"),
                ("Access-Control-Allow-Credentials", "true"),
            ],
        )
        .await;
        let transport = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let first = document(&transport, "https://first.test/page", "https://first.test");
        let second = first.fork_for_document(DocumentFetchContext::new(
            WindowDocumentOwner::Frame(FrameDocumentTaskOwner::new(
                FrameSchedulerLaneId(7),
                LocalWindowId(11),
                DocumentId(2),
            )),
            Url::parse("https://second.test/page").unwrap(),
            Url::parse("https://second.test/base").unwrap(),
            "https://second.test",
        ));
        for _ in 0..2 {
            fetch_script(
                &first,
                script_request(
                    &server.url,
                    RequestMode::Cors,
                    RequestCredentialsMode::Include,
                ),
                callback,
            )
            .await
            .unwrap();
        }
        assert_eq!(
            server.request_count(),
            1,
            "same Document should reuse its authorized response"
        );
        let result = fetch_script(
            &second,
            script_request(
                &server.url,
                RequestMode::Cors,
                RequestCredentialsMode::Include,
            ),
            callback,
        )
        .await;
        assert!(
            result.is_err(),
            "another Document must perform its own CORS check"
        );

        let server = ScriptServer::new(200, &[]).await;
        fetch_script(
            &first,
            script_request(
                &server.url,
                RequestMode::NoCors,
                RequestCredentialsMode::Include,
            ),
            callback,
        )
        .await
        .unwrap();
        let result = fetch_script(
            &first,
            script_request(
                &server.url,
                RequestMode::Cors,
                RequestCredentialsMode::Include,
            ),
            callback,
        )
        .await;
        assert!(
            result.is_err(),
            "a classic no-cors cache entry must not authorize a module"
        );
    }
}

#[tokio::test]
async fn script_cors_rejects_a_redirect_before_requesting_its_target() {
    for callback in [false, true] {
        let final_server = ScriptServer::new(200, &[("Access-Control-Allow-Origin", "*")]).await;
        let redirect = ScriptServer::new(302, &[("Location", final_server.url.as_str())]).await;
        let transport = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let document = document(&transport, "https://page.test/page", "https://page.test");
        let result = fetch_script(
            &document,
            script_request(
                &redirect.url,
                RequestMode::Cors,
                RequestCredentialsMode::SameOrigin,
            ),
            callback,
        )
        .await;
        assert!(
            result.is_err(),
            "missing CORS headers on an intermediate redirect must reject"
        );
        assert_eq!(
            final_server.request_count(),
            0,
            "blocked redirect target must not be fetched"
        );
    }
}

#[tokio::test]
async fn script_cors_cross_origin_redirects_taint_the_origin() {
    for callback in [false, true] {
        for (allow_origin, allowed) in [("null", true), ("https://page.test", false)] {
            let final_server =
                ScriptServer::new(200, &[("Access-Control-Allow-Origin", allow_origin)]).await;
            let redirect = ScriptServer::new(
                302,
                &[
                    ("Location", final_server.url.as_str()),
                    ("Access-Control-Allow-Origin", "https://page.test"),
                ],
            )
            .await;
            let transport = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let document = document(&transport, "https://page.test/page", "https://page.test");
            let result = fetch_script(
                &document,
                script_request(
                    &redirect.url,
                    RequestMode::Cors,
                    RequestCredentialsMode::SameOrigin,
                ),
                callback,
            )
            .await;
            assert_eq!(result.is_ok(), allowed, "{result:?}");
            assert_eq!(redirect.last_origin().as_deref(), Some("https://page.test"));
            assert_eq!(final_server.last_origin().as_deref(), Some("null"));
        }
    }
}

#[tokio::test]
async fn script_cors_redirects_drop_authorization_apply_referrer_policy_and_preserve_fragments() {
    for callback in [false, true] {
        let final_server = ScriptServer::new(200, &[("Access-Control-Allow-Origin", "*")]).await;
        let redirect = ScriptServer::new(
            302,
            &[
                ("Location", final_server.url.as_str()),
                ("Referrer-Policy", "no-referrer"),
            ],
        )
        .await;
        let transport = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let origin = moli_url::origin_ascii_serialization(&redirect.url);
        let document = document(&transport, redirect.url.as_str(), &origin);
        let mut request = script_request(
            &redirect.url,
            RequestMode::Cors,
            RequestCredentialsMode::SameOrigin,
        );
        request.url.set_fragment(Some("module-fragment"));
        request.request_headers = vec![
            ("Authorization".to_owned(), "Bearer test-only".to_owned()),
            ("X-Embedder".to_owned(), "preserved".to_owned()),
        ];
        let response = fetch_script(&document, request, callback).await.unwrap();
        assert_eq!(response.final_url.fragment(), Some("module-fragment"));
        assert_eq!(redirect.request_count(), 1);
        assert_eq!(
            final_server.request_count(),
            1,
            "browser-added script headers must not trigger preflight"
        );
        assert_eq!(
            redirect.last_header("authorization").as_deref(),
            Some("Bearer test-only")
        );
        assert_eq!(final_server.last_header("authorization"), None);
        assert_eq!(final_server.last_header("referer"), None);
        assert_eq!(
            final_server.last_header("x-embedder").as_deref(),
            Some("preserved")
        );
    }
}

#[tokio::test]
async fn script_cors_redirects_reject_non_http_schemes_and_cross_origin_url_credentials() {
    for callback in [false, true] {
        for target in [
            "data:text/javascript,export default 1",
            "http://user:pass@other.test/module.js",
        ] {
            let redirect = ScriptServer::new(302, &[("Location", target)]).await;
            let transport = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let origin = moli_url::origin_ascii_serialization(&redirect.url);
            let document = document(&transport, redirect.url.as_str(), &origin);
            let result = fetch_script(
                &document,
                script_request(
                    &redirect.url,
                    RequestMode::Cors,
                    RequestCredentialsMode::SameOrigin,
                ),
                callback,
            )
            .await;
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("CORS redirect"));
        }
    }
}

#[tokio::test]
async fn script_cors_uses_inherited_and_opaque_document_origins_after_transport_replacement() {
    for callback in [false, true] {
        for origin in ["https://creator.test", "null"] {
            let server = ScriptServer::new(200, &[("Access-Control-Allow-Origin", origin)]).await;
            let transport = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let replacement = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let document = document(&transport, "about:srcdoc", origin)
                .with_replacement_transport(replacement.handle());
            fetch_script(
                &document,
                script_request(
                    &server.url,
                    RequestMode::Cors,
                    RequestCredentialsMode::SameOrigin,
                ),
                callback,
            )
            .await
            .unwrap();
            assert_eq!(server.last_origin().as_deref(), Some(origin));
        }
    }
}

#[test]
fn script_cors_memory_cache_key_separates_origins_and_request_modes() {
    let url = Url::parse("https://script.test/module.js").unwrap();
    let first = script_request(&url, RequestMode::Cors, RequestCredentialsMode::Include)
        .with_request_origin(moli_url::WebOrigin::from_ascii_serialization(
            "https://first.test",
        ));
    let second = first
        .clone()
        .with_request_origin(moli_url::WebOrigin::from_ascii_serialization(
            "https://second.test",
        ));
    assert_ne!(
        super::super::script_text_cache_key(&first),
        super::super::script_text_cache_key(&second)
    );
    assert_ne!(
        super::super::script_text_cache_key(&first),
        super::super::script_text_cache_key(&first.clone().with_request_mode(RequestMode::NoCors))
    );
}
