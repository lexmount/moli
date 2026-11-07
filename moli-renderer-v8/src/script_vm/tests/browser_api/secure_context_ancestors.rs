use crate::network::ResourceRequestClient;
use crate::{JsRuntime, RendererPageCommand, RendererPageReply};
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::{TlsAcceptor, rustls};

const CHILD: &str = r#"<!doctype html><body><script>
parent.postMessage({kind:'window',secure:isSecureContext,subtle:typeof crypto.subtle}, '*');
// Author-visible properties must not supply the worker's creator policy.
Object.defineProperty(globalThis,'isSecureContext',{value:!isSecureContext,configurable:true});
const worker = new Worker('/worker.js');
worker.onmessage = e => parent.postMessage({kind:'worker',...e.data}, '*');
worker.onerror = e => parent.postMessage({kind:'error',message:e.message}, '*');
const shared = new SharedWorker('/shared.js');
shared.port.onmessage = e => parent.postMessage(e.data, '*');
shared.onerror = e => parent.postMessage({kind:'error',message:e.message}, '*');
shared.port.start();
</script>"#;
const WORKER: &str = "postMessage({secure:isSecureContext,subtle:typeof crypto.subtle});close();";
const SHARED: &str = r#"
onconnect = e => {
  const port = e.ports[0]; port.start();
  port.postMessage({kind:'shared',secure:isSecureContext,subtle:typeof crypto.subtle});
  const worker = new Worker('/worker.js');
  worker.onmessage = e => port.postMessage({kind:'nested',...e.data});
  worker.onerror = e => port.postMessage({kind:'error',message:e.message});
};
"#;

const POPUP: &str = r#"<!doctype html><body><script>
addEventListener('message', e => opener.postMessage(e.data, '*'));
</script><iframe src='/child'></iframe>"#;

struct SecureChildFixture {
    _directory: tempfile::TempDir,
    ca_file: std::path::PathBuf,
    url: String,
    server: tokio::task::JoinHandle<()>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Drop for SecureChildFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn secure_child_fixture() -> SecureChildFixture {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::new()).unwrap();
    ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Moli secure context test CA");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let key = KeyPair::generate().unwrap();
    let certificate = CertificateParams::new(vec!["127.0.0.1".to_owned()])
        .unwrap()
        .signed_by(&key, &ca, &ca_key)
        .unwrap();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "https://127.0.0.1:{}/child",
        listener.local_addr().unwrap().port()
    );
    let directory = tempfile::tempdir().unwrap();
    let ca_file = directory.path().join("ca.pem");
    std::fs::write(&ca_file, ca.pem()).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let request_log = requests.clone();
    let server = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            request_log.lock().push("TCP accepted".to_owned());
            let acceptor = acceptor.clone();
            let request_log = request_log.clone();
            tokio::spawn(async move {
                let mut stream = match acceptor.accept(socket).await {
                    Ok(stream) => stream,
                    Err(error) => {
                        request_log.lock().push(format!("TLS error: {error}"));
                        return;
                    }
                };
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let Ok(length) = stream.read(&mut buffer).await else {
                        return;
                    };
                    if length == 0 || request.len() > 8192 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..length]);
                }
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                request_log.lock().push(path.to_owned());
                let (content_type, body) = match path {
                    "/child" => ("text/html", CHILD),
                    "/popup" => ("text/html", POPUP),
                    "/worker.js" => ("application/javascript", WORKER),
                    "/shared.js" => ("application/javascript", SHARED),
                    "/probe" => ("text/plain", "fixture ready"),
                    _ => ("text/plain", "unexpected resource"),
                };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len(),
                );
                let _ = stream.write_all(header.as_bytes()).await;
                let _ = stream.write_all(body.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    SecureChildFixture {
        _directory: directory,
        ca_file,
        url,
        server,
        requests,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn https_frame_and_worker_settings_follow_all_ancestors_and_ignore_public_spoofing() {
    let fixture = secure_child_fixture().await;
    for (root, secure, popup) in [
        ("http://insecure-ancestor.test/", false, false),
        ("https://secure-ancestor.test/", true, false),
        ("http://localhost/", true, false),
        ("http://insecure-opener.test/", true, true),
    ] {
        let mut config = moli_fetch::FetchConfig::default();
        config.set_tls_credentials(Some(fixture.ca_file.clone()), None, None, None);
        let loader = ResourceRequestClient::new(&config).unwrap();
        let fixture_url = url::Url::parse(&fixture.url).unwrap();
        loader
            .fetch(
                moli_fetch::Request::get(&fixture.url.replace("/child", "/probe"))
                    .unwrap()
                    .with_request_origin(moli_url::WebOrigin::from_url(&fixture_url)),
            )
            .await
            .expect("the fixture CA and HTTPS listener must work before testing browser contexts");
        let runtime = JsRuntime::initialize();
        let root_url = url::Url::parse(root).unwrap();
        let child = if popup {
            format!(
                "<script>open({});</script>",
                serde_json::to_string(&fixture.url.replace("/child", "/popup")).unwrap()
            )
        } else {
            format!(r#"<iframe src="{}"></iframe>"#, fixture.url)
        };
        let (mut page, _, _, _artifacts, download) = runtime
            .create_html_page_from_response(
                root_url.clone(),
                root_url,
                None,
                false,
                0,
                200,
                vec![("content-type".to_owned(), b"text/html".to_vec())],
                &loader,
                crate::RendererWebStorageHandles::ephemeral(),
                format!(
                    r#"<!doctype html><body><script>
globalThis.secureAncestorResults = [];
addEventListener('message', e => secureAncestorResults.push(e.data));
</script>{child}"#
                ),
                crate::RendererDocumentOptions::default(),
            )
            .await
            .unwrap();
        assert!(download.is_none());
        let delivered = page.run_async_command(RendererPageCommand::WaitForScriptTruthy {
            expression: "secureAncestorResults.length === 4 || secureAncestorResults.some(r => r.kind === 'error')".to_owned(),
            timeout_ms: 2_000,
            loader: loader.clone(),
        }).await;
        let (reply, _) = page
            .run_async_command(RendererPageCommand::EvaluateExpression {
                expression:
                    "JSON.stringify(secureAncestorResults.sort((a,b) => a.kind < b.kind ? -1 : 1))"
                        .to_owned(),
                await_promise: false,
            })
            .await
            .unwrap();
        let RendererPageReply::RuntimeEvaluationResult(reply) = reply else {
            panic!("expected evaluated secure context results");
        };
        let payload = reply.into_protocol_payload();
        let result: serde_json::Value =
            serde_json::from_str(payload["value"].as_str().unwrap()).unwrap();
        assert!(
            delivered.is_ok(),
            "creator {root}, popup={popup}: {:?}; messages={result}; requests={:?}",
            delivered.as_ref().err(),
            fixture.requests.lock()
        );
        let subtle = if secure { "object" } else { "undefined" };
        assert_eq!(
            result,
            serde_json::json!([
                {"kind":"nested","secure":secure,"subtle":subtle},
                {"kind":"shared","secure":secure,"subtle":subtle},
                {"kind":"window","secure":secure,"subtle":subtle},
                {"kind":"worker","secure":secure,"subtle":subtle},
            ]),
            "creator {root}, popup={popup}"
        );
        page.close_async().await.unwrap();
    }
}
