use crate::{
    WptManifestOrigin, WptManifestTest,
    meta::{extract_wpt_meta_script_references, resolve_wpt_static_resource_reference},
};
use anyhow::{Context, Result};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{
        Path, Query, State,
        ws::{Message as WebSocketMessage, WebSocket, WebSocketUpgrade},
    },
    http::{
        HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri,
        header::{
            ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS,
            ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
            ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_REQUEST_HEADERS,
            ACCESS_CONTROL_REQUEST_METHOD, CONTENT_TYPE, COOKIE, HOST, LOCATION, ORIGIN,
            SET_COOKIE, VARY,
        },
    },
    response::{IntoResponse, Redirect, Response},
    routing::{any, get, post},
    serve::Listener,
};
use parking_lot::Mutex;
use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Component, Path as StdPath, PathBuf},
    sync::{Arc, OnceLock},
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::{Duration, sleep},
};
use tokio_rustls::{TlsAcceptor, server::TlsStream};
use tokio_stream::wrappers::ReceiverStream;

const COMPILED_WPT_FIXTURE_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/wpt");
const WPT_STATIC_FIXTURE_TOP_LEVEL_DIRS: &[&str] = &["assets", "ported", "resources", "upstream"];
const WPT_BROWSER_HOST: &str = "localhost";
const WPT_INSECURE_HOST: &str = "0.0.0.0";
const WPT_REMOTE_HOST: &str = "127.0.0.1";
const WPT_WINDOW_WRAPPER_QUERY: &str = "moli-wpt-window-wrapper";
const WPT_WORKER_ENTRY_QUERY: &str = "moli-wpt-worker-entry";
const WPT_SHARED_WORKER_WRAPPER_QUERY: &str = "moli-wpt-shared-worker-wrapper";
const WPT_WORKER_WRAPPER_QUERY: &str = "moli-wpt-worker-wrapper";
const WPT_XHR_TEXT_BODY: &str = "1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890";
const WPT_XHR_JSON_BODY: &str = r#"{"over":"9000!!!","updated_at":1765867200000}"#;
const WPT_WORKER_SLOW_BODY: &str = r#"postMessage("loaded");
onmessage = (event) => {
  postMessage(`pong:${event.data}`);
};
"#;
const WPT_WORKER_IMPORTSCRIPTS_ARGS_WORKER_BODY: &str = r#"onmessage = function (event) {
  switch (event.data) {
    case "symbol":
      try {
        importScripts(Symbol("worker.js"));
        postMessage({ name: "unexpected-success" });
      } catch (error) {
        postMessage({ name: error && error.name });
      }
      close();
      return;
    case "invalid-url":
      try {
        importScripts(
          "data:text/javascript,globalThis.__lmImportScriptsRan=true",
          "http://foo bar"
        );
        postMessage({ name: "unexpected-success", ran: globalThis.__lmImportScriptsRan === true });
      } catch (error) {
        postMessage({
          name: error && error.name,
          ran: globalThis.__lmImportScriptsRan === true,
        });
      }
      close();
      return;
    case "stringify":
      importScripts(undefined, null, 1);
      postMessage({
        undefinedLoaded: globalThis.__undefinedLoaded === true,
        nullLoaded: globalThis.__nullLoaded === true,
        oneLoaded: globalThis.__oneLoaded === true,
      });
      close();
      return;
    default:
      postMessage({
        name: "unexpected-scenario",
        scenario: event.data,
      });
      close();
  }
};
"#;
const WPT_WORKER_IMPORTSCRIPTS_UNDEFINED_BODY: &str = "globalThis.__undefinedLoaded = true;\n";
const WPT_WORKER_IMPORTSCRIPTS_NULL_BODY: &str = "globalThis.__nullLoaded = true;\n";
const WPT_WORKER_IMPORTSCRIPTS_ONE_BODY: &str = "globalThis.__oneLoaded = true;\n";

#[derive(Default)]
struct AbortObservationRegistry {
    states: Mutex<HashMap<String, Option<bool>>>,
}

impl AbortObservationRegistry {
    fn mark_pending(&self, token: &str) {
        self.states.lock().insert(token.to_owned(), None);
    }

    fn mark_complete(&self, token: &str, disconnected: bool) {
        self.states
            .lock()
            .insert(token.to_owned(), Some(disconnected));
    }

    fn lookup(&self, token: &str) -> Option<Option<bool>> {
        self.states.lock().get(token).copied()
    }
}

#[derive(Default)]
struct CorsPreflightObservationRegistry {
    states: Mutex<HashSet<String>>,
}

impl CorsPreflightObservationRegistry {
    fn mark_preflight(&self, token: &str) {
        self.states.lock().insert(token.to_owned());
    }

    fn take_preflighted(&self, token: &str) -> bool {
        self.states.lock().remove(token)
    }
}

#[derive(Default)]
struct CspReportObservationRegistry {
    reports: Mutex<HashMap<String, Vec<CspReportObservation>>>,
}

struct CspReportObservation {
    content_type: Option<String>,
    body: String,
}

impl CspReportObservationRegistry {
    fn push(&self, token: &str, report: CspReportObservation) {
        self.reports
            .lock()
            .entry(token.to_owned())
            .or_default()
            .push(report);
    }

    fn take(&self, token: &str) -> Vec<CspReportObservation> {
        self.reports.lock().remove(token).unwrap_or_default()
    }
}

#[derive(Clone)]
struct WptFixtureRuntimeState {
    primary_addr: std::net::SocketAddr,
    secondary_addr: std::net::SocketAddr,
    primary_https_addr: std::net::SocketAddr,
    secondary_https_addr: std::net::SocketAddr,
    abort_observations: Arc<AbortObservationRegistry>,
    cors_preflight_observations: Arc<CorsPreflightObservationRegistry>,
    csp_report_observations: Arc<CspReportObservationRegistry>,
}

struct WptTlsListener {
    listener: TcpListener,
    acceptor: TlsAcceptor,
}

impl WptTlsListener {
    fn new(listener: TcpListener, acceptor: TlsAcceptor) -> Self {
        Self { listener, acceptor }
    }
}

impl Listener for WptTlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, addr) = match self.listener.accept().await {
                Ok(accepted) => accepted,
                Err(_) => {
                    sleep(Duration::from_millis(1)).await;
                    continue;
                }
            };
            match self.acceptor.accept(stream).await {
                Ok(stream) => return (stream, addr),
                Err(_) => continue,
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

fn wpt_tls_acceptor() -> Result<TlsAcceptor> {
    use tokio_rustls::rustls::{
        ServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    };

    let cert = rcgen::generate_simple_self_signed(vec![
        WPT_BROWSER_HOST.to_owned(),
        WPT_REMOTE_HOST.to_owned(),
    ])
    .context("failed to generate WPT fixture TLS certificate")?;
    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()));
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .context("failed to build WPT fixture TLS config")?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub struct WptFixtureServer {
    addr: std::net::SocketAddr,
    shutdown_txs: Vec<oneshot::Sender<()>>,
    tasks: Vec<JoinHandle<()>>,
}

impl WptFixtureServer {
    pub async fn spawn() -> Result<Self> {
        let primary_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("failed to bind primary WPT fixture server")?;
        let addr = primary_listener
            .local_addr()
            .context("failed to read primary WPT fixture server address")?;
        let secondary_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("failed to bind secondary WPT fixture server")?;
        let secondary_addr = secondary_listener
            .local_addr()
            .context("failed to read secondary WPT fixture server address")?;
        let primary_https_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("failed to bind primary HTTPS WPT fixture server")?;
        let primary_https_addr = primary_https_listener
            .local_addr()
            .context("failed to read primary HTTPS WPT fixture server address")?;
        let secondary_https_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("failed to bind secondary HTTPS WPT fixture server")?;
        let secondary_https_addr = secondary_https_listener
            .local_addr()
            .context("failed to read secondary HTTPS WPT fixture server address")?;
        let tls_acceptor = wpt_tls_acceptor()?;
        let primary_tls_acceptor = tls_acceptor.clone();
        let secondary_tls_acceptor = tls_acceptor;
        let runtime_state = WptFixtureRuntimeState {
            primary_addr: addr,
            secondary_addr,
            primary_https_addr,
            secondary_https_addr,
            abort_observations: Arc::new(AbortObservationRegistry::default()),
            cors_preflight_observations: Arc::new(CorsPreflightObservationRegistry::default()),
            csp_report_observations: Arc::new(CspReportObservationRegistry::default()),
        };
        let app = wpt_fixture_app(runtime_state);
        let primary_app = app.clone();
        let primary_https_app = app.clone();
        let secondary_https_app = app.clone();
        let (primary_shutdown_tx, primary_shutdown_rx) = oneshot::channel();
        let (secondary_shutdown_tx, secondary_shutdown_rx) = oneshot::channel();
        let (primary_https_shutdown_tx, primary_https_shutdown_rx) = oneshot::channel();
        let (secondary_https_shutdown_tx, secondary_https_shutdown_rx) = oneshot::channel();

        let primary_task = tokio::spawn(async move {
            if let Err(error) = axum::serve(primary_listener, primary_app)
                .with_graceful_shutdown(async {
                    let _ = primary_shutdown_rx.await;
                })
                .await
            {
                panic!("WPT fixture server failed: {error}");
            }
        });
        let secondary_task = tokio::spawn(async move {
            if let Err(error) = axum::serve(secondary_listener, app)
                .with_graceful_shutdown(async {
                    let _ = secondary_shutdown_rx.await;
                })
                .await
            {
                panic!("WPT fixture server failed: {error}");
            }
        });
        let primary_https_task = tokio::spawn(async move {
            let listener = WptTlsListener::new(primary_https_listener, primary_tls_acceptor);
            if let Err(error) = axum::serve(listener, primary_https_app)
                .with_graceful_shutdown(async {
                    let _ = primary_https_shutdown_rx.await;
                })
                .await
            {
                panic!("WPT HTTPS fixture server failed: {error}");
            }
        });
        let secondary_https_task = tokio::spawn(async move {
            let listener = WptTlsListener::new(secondary_https_listener, secondary_tls_acceptor);
            if let Err(error) = axum::serve(listener, secondary_https_app)
                .with_graceful_shutdown(async {
                    let _ = secondary_https_shutdown_rx.await;
                })
                .await
            {
                panic!("WPT HTTPS fixture server failed: {error}");
            }
        });

        Ok(Self {
            addr,
            shutdown_txs: vec![
                primary_shutdown_tx,
                secondary_shutdown_tx,
                primary_https_shutdown_tx,
                secondary_https_shutdown_tx,
            ],
            tasks: vec![
                primary_task,
                secondary_task,
                primary_https_task,
                secondary_https_task,
            ],
        })
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}:{}{}", WPT_BROWSER_HOST, self.addr.port(), path)
    }

    pub fn case_url(&self, test: &WptManifestTest) -> String {
        wpt_case_url(
            self.addr,
            &test.local_path,
            &test.global,
            &test.query,
            test.origin,
        )
    }

    pub fn fixture_url(&self, local_path: &str) -> String {
        wpt_fixture_url(self.addr, local_path)
    }

    #[cfg(test)]
    pub(crate) fn for_test_addr(addr: std::net::SocketAddr) -> Self {
        Self {
            addr,
            shutdown_txs: Vec::new(),
            tasks: Vec::new(),
        }
    }

    pub async fn shutdown(mut self) {
        while let Some(shutdown_tx) = self.shutdown_txs.pop() {
            let _ = shutdown_tx.send(());
        }

        while let Some(task) = self.tasks.pop() {
            let _ = task.await;
        }
    }
}

fn wpt_fixture_app(runtime_state: WptFixtureRuntimeState) -> Router {
    Router::new()
        .route("/wpt/runtime/cookies/http-set", get(wpt_cookie_http_set))
        .route(
            "/wpt/runtime/cookies/http-check",
            get(wpt_cookie_http_check),
        )
        .route(
            "/wpt/runtime/cookies/path-scope/set",
            get(wpt_cookie_path_scope_set),
        )
        .route(
            "/wpt/runtime/cookies/path-scope/check",
            get(wpt_cookie_path_scope_check),
        )
        .route(
            "/wpt/runtime/cookies/path-scope-extra/check",
            get(wpt_cookie_path_scope_extra_check),
        )
        .route(
            "/wpt/runtime/cookies/invalid-domain/set",
            get(wpt_cookie_invalid_domain_set),
        )
        .route(
            "/wpt/runtime/cookies/invalid-domain/check",
            get(wpt_cookie_invalid_domain_check),
        )
        .route(
            "/wpt/runtime/cookies/replace/red",
            get(wpt_cookie_replace_red),
        )
        .route(
            "/wpt/runtime/cookies/replace/blue",
            get(wpt_cookie_replace_blue),
        )
        .route(
            "/wpt/runtime/cookies/replace/check",
            get(wpt_cookie_replace_check),
        )
        .route(
            "/wpt/runtime/cookies/samesite/set",
            get(wpt_cookie_samesite_set),
        )
        .route(
            "/wpt/runtime/cookies/samesite/check",
            get(wpt_cookie_samesite_check),
        )
        .route(
            "/wpt/runtime/cookies/redirect-chain/start",
            get(wpt_cookie_redirect_chain_start),
        )
        .route(
            "/wpt/runtime/cookies/redirect-chain/middle",
            get(wpt_cookie_redirect_chain_middle),
        )
        .route(
            "/wpt/runtime/cookies/redirect-chain/final",
            get(wpt_cookie_redirect_chain_final),
        )
        .route("/wpt/runtime/xhr/text", get(wpt_xhr_text))
        .route("/wpt/runtime/xhr/html-text", get(wpt_xhr_html_text))
        .route("/wpt/runtime/xhr/json", get(wpt_xhr_json))
        .route("/wpt/runtime/xhr/binary", get(wpt_xhr_binary))
        .route("/wpt/runtime/xhr/redirect", get(wpt_xhr_redirect))
        .route("/wpt/runtime/xhr/404", get(wpt_xhr_404))
        .route("/wpt/runtime/xhr/500", get(wpt_xhr_500))
        .route("/wpt/runtime/xhr/slow", get(wpt_xhr_slow))
        .route("/wpt/runtime/xhr/very-slow", get(wpt_xhr_very_slow))
        .route("/wpt/runtime/xhr/empty", get(wpt_xhr_empty))
        .route(
            "/wpt/runtime/xhr/abort-observing/{token}",
            get(wpt_xhr_abort_observing),
        )
        .route(
            "/wpt/runtime/xhr/abort-observing-status/{token}",
            get(wpt_xhr_abort_observing_status),
        )
        .route(
            "/wpt/runtime/fetch/abort-observing/{token}",
            get(wpt_fetch_abort_observing),
        )
        .route(
            "/wpt/runtime/fetch/abort-observing-status/{token}",
            get(wpt_fetch_abort_observing_status),
        )
        .route(
            "/wpt/runtime/xhr/request-headers",
            get(wpt_xhr_request_headers),
        )
        .route(
            "/wpt/runtime/fetch/request-headers",
            get(wpt_xhr_request_headers),
        )
        .route("/wpt/runtime/xhr/echo-body", post(wpt_xhr_echo_body))
        .route("/wpt/runtime/xhr/echo-body-hex", post(wpt_echo_body_hex))
        .route("/wpt/runtime/cors/allow", get(wpt_cors_allow))
        .route("/wpt/runtime/cors/deny", get(wpt_cors_deny))
        .route("/wpt/runtime/cors/exposed", get(wpt_cors_exposed_headers))
        .route(
            "/wpt/runtime/cors/exposed/wildcard",
            get(wpt_cors_exposed_headers_wildcard),
        )
        .route(
            "/wpt/runtime/cors/exposed/wildcard-credentials",
            get(wpt_cors_exposed_headers_wildcard_credentials),
        )
        .route(
            "/wpt/runtime/cors/preflight/allow",
            any(wpt_cors_preflight_allow),
        )
        .route(
            "/wpt/runtime/cors/preflight/deny-method",
            any(wpt_cors_preflight_deny_method),
        )
        .route(
            "/wpt/runtime/cors/preflight/deny-header",
            any(wpt_cors_preflight_deny_header),
        )
        .route(
            "/wpt/runtime/cors/preflight/redirect-to-allow",
            any(wpt_cors_preflight_redirect_to_allow),
        )
        .route(
            "/wpt/runtime/cors/preflight/redirect-to-preflight-required",
            any(wpt_cors_preflight_redirect_to_preflight_required),
        )
        .route(
            "/wpt/runtime/cors/preflight/preflight-required-final",
            any(wpt_cors_preflight_required_final),
        )
        .route(
            "/wpt/runtime/cors/redirect/to-credentials-allow",
            get(wpt_cors_redirect_to_credentials_allow),
        )
        .route(
            "/wpt/runtime/cors/redirect/to-same-origin-echo",
            get(wpt_cors_redirect_to_same_origin_echo),
        )
        .route(
            "/wpt/runtime/cors/redirect/to-alt-to-same-origin-echo",
            get(wpt_cors_redirect_to_alt_to_same_origin_echo),
        )
        .route(
            "/wpt/runtime/cors/redirect/to-cross-site-to-same-origin-echo",
            get(wpt_cors_redirect_to_cross_site_to_same_origin_echo),
        )
        .route(
            "/wpt/runtime/cors/redirect/303-to-method-echo",
            any(wpt_cors_redirect_303_to_method_echo),
        )
        .route(
            "/wpt/runtime/cors/redirect/307-to-method-echo",
            any(wpt_cors_redirect_307_to_method_echo),
        )
        .route(
            "/wpt/runtime/cors/redirect/method-echo",
            any(wpt_cors_redirect_method_echo),
        )
        .route(
            "/wpt/runtime/cors/redirect/same-origin-echo",
            get(wpt_cors_redirect_same_origin_echo),
        )
        .route(
            "/wpt/runtime/cors/credentials/allow",
            get(wpt_cors_credentials_allow),
        )
        .route(
            "/wpt/runtime/cors/credentials/wildcard",
            get(wpt_cors_credentials_wildcard),
        )
        .route("/wpt/runtime/forms/submit-echo", any(wpt_form_submit_echo))
        .route("/wpt/runtime/websocket/echo", get(wpt_websocket_echo))
        .route(
            "/wpt/runtime/websocket/cookie-set",
            get(wpt_websocket_cookie_set),
        )
        .route(
            "/wpt/runtime/websocket/cookie-echo",
            get(wpt_websocket_cookie_echo),
        )
        .route(
            "/wpt/runtime/websocket/subprotocol",
            get(wpt_websocket_subprotocol),
        )
        .route(
            "/wpt/runtime/websocket/server-close",
            get(wpt_websocket_server_close),
        )
        .route(
            "/wpt/runtime/websocket/slow-handshake",
            get(wpt_websocket_slow_handshake),
        )
        .route(
            "/wpt/runtime/websocket/plain-200",
            get(wpt_websocket_plain_200),
        )
        .route(
            "/wpt/runtime/websocket/response-cookie",
            get(wpt_websocket_response_cookie),
        )
        .route(
            "/wpt/runtime/worker/module-redirect/start.js",
            get(wpt_worker_module_redirect_start),
        )
        .route(
            "/wpt/runtime/worker/module-redirect/fragment-start.js",
            get(wpt_worker_module_redirect_fragment_start),
        )
        .route(
            "/wpt/runtime/worker/module-credentials/set-cookie",
            get(wpt_worker_module_credentials_cookie_set),
        )
        .route(
            "/wpt/runtime/worker/module-credentials/check-cookie",
            get(wpt_worker_module_credentials_cookie_check),
        )
        .route(
            "/wpt/runtime/worker/module-credentials/main.js",
            get(wpt_worker_module_credentials_main),
        )
        .route(
            "/wpt/runtime/worker/module-credentials/dependency.js",
            get(wpt_worker_module_credentials_dependency),
        )
        .route(
            "/wpt/runtime/sharedworker/script-fetch-policy/set-cookie",
            get(wpt_shared_worker_script_fetch_policy_cookie_set),
        )
        .route(
            "/wpt/runtime/sharedworker/script-fetch-policy/set-remote-cookie",
            get(wpt_shared_worker_script_fetch_policy_remote_cookie_set),
        )
        .route(
            "/wpt/runtime/sharedworker/script-fetch-policy/report-cookie.js",
            get(wpt_shared_worker_script_fetch_policy_report_cookie),
        )
        .route(
            "/wpt/runtime/sharedworker/script-fetch-policy/report-import-cookie-main.js",
            get(wpt_shared_worker_script_fetch_policy_report_import_cookie_main),
        )
        .route(
            "/wpt/runtime/sharedworker/script-fetch-policy/report-import-cookie-dependency.js",
            get(wpt_shared_worker_script_fetch_policy_report_import_cookie_dependency),
        )
        .route(
            "/wpt/runtime/sharedworker/script-fetch-policy/report-import-cookie-remote-dependency.js",
            get(wpt_shared_worker_script_fetch_policy_report_import_cookie_remote_dependency),
        )
        .route(
            "/wpt/runtime/sharedworker/script-fetch-policy/referrer-policy-main.js",
            get(wpt_shared_worker_script_fetch_policy_referrer_policy_main),
        )
        .route(
            "/wpt/runtime/sharedworker/script-fetch-policy/report-referrer-dependency.js",
            get(wpt_shared_worker_script_fetch_policy_report_referrer_dependency),
        )
        .route(
            "/wpt/runtime/sharedworker/script-fetch-policy/cross-origin-redirect-start.js",
            get(wpt_shared_worker_script_fetch_policy_cross_origin_redirect_start),
        )
        .route(
            "/wpt/runtime/sharedworker/script-fetch-policy/cross-origin-target.js",
            get(wpt_shared_worker_script_fetch_policy_cross_origin_target),
        )
        .route(
            "/wpt/runtime/csp/reporting/target.txt",
            get(wpt_csp_reporting_target),
        )
        .route(
            "/wpt/runtime/csp/reporting/collect",
            post(wpt_csp_reporting_collect),
        )
        .route(
            "/wpt/runtime/csp/reporting/take",
            get(wpt_csp_reporting_take),
        )
        .route(
            "/wpt/runtime/worker/module-json-mime/config.data",
            get(wpt_worker_module_json_mime_config),
        )
        .route(
            "/wpt/runtime/worker/module-json-mime/json-with-charset.data",
            get(wpt_worker_module_json_mime_with_charset),
        )
        .route(
            "/wpt/runtime/worker/module-json-mime/javascript.json",
            get(wpt_worker_module_json_mime_javascript),
        )
        .route(
            "/wpt/runtime/worker/module-json-mime/plain-json.json",
            get(wpt_worker_module_json_mime_plain_json),
        )
        .route(
            "/wpt/runtime/worker/importscripts/cross-origin-target.js",
            get(wpt_worker_importscripts_cross_origin_target),
        )
        .route(
            "/wpt/runtime/worker/importscripts/redirect-to-cross-origin.js",
            get(wpt_worker_importscripts_redirect_to_cross_origin),
        )
        .route(
            "/wpt/runtime/worker/importscripts/args-worker.js",
            get(wpt_worker_importscripts_args_worker),
        )
        .route(
            "/wpt/runtime/worker/importscripts/undefined",
            get(wpt_worker_importscripts_undefined),
        )
        .route(
            "/wpt/runtime/worker/importscripts/null",
            get(wpt_worker_importscripts_null),
        )
        .route(
            "/wpt/runtime/worker/importscripts/1",
            get(wpt_worker_importscripts_one),
        )
        .route("/wpt/runtime/worker/slow.js", get(wpt_worker_slow_script))
        .route(
            "/wpt/runtime/location/port-config.js",
            get(wpt_location_port_config),
        )
        .route(
            "/wpt/runtime/fixture-server-config.js",
            get(wpt_fixture_server_config),
        )
        .route(
            "/wpt/runtime/fetch/credentials/request-cookie/set",
            get(wpt_fetch_credentials_request_cookie_set),
        )
        .route("/wpt/runtime/fetch/echo-body-hex", post(wpt_echo_body_hex))
        .route(
            "/wpt/runtime/fetch/binary-response",
            get(wpt_fetch_binary_response),
        )
        .route(
            "/wpt/runtime/fetch/credentials/request-cookie/check",
            get(wpt_fetch_credentials_request_cookie_check),
        )
        .route(
            "/wpt/runtime/fetch/credentials/default-response-cookie/set",
            get(wpt_fetch_credentials_default_response_cookie_set),
        )
        .route(
            "/wpt/runtime/fetch/credentials/default-response-cookie/check",
            get(wpt_fetch_credentials_default_response_cookie_check),
        )
        .route(
            "/wpt/runtime/fetch/credentials/include-response-cookie/set",
            get(wpt_fetch_credentials_include_response_cookie_set),
        )
        .route(
            "/wpt/runtime/fetch/credentials/include-response-cookie/check",
            get(wpt_fetch_credentials_include_response_cookie_check),
        )
        .route(
            "/wpt/runtime/fetch/credentials/omit-response-cookie/set",
            get(wpt_fetch_credentials_omit_response_cookie_set),
        )
        .route(
            "/wpt/runtime/fetch/credentials/omit-response-cookie/check",
            get(wpt_fetch_credentials_omit_response_cookie_check),
        )
        .route(
            "/wpt/runtime/fetch/credentials/streaming/{mode}/set",
            get(wpt_fetch_credentials_streaming_response_cookie_set),
        )
        .route(
            "/wpt/runtime/fetch/credentials/streaming/{mode}/check",
            get(wpt_fetch_credentials_streaming_response_cookie_check),
        )
        .route(
            "/wpt/runtime/xhr/credentials/request-cookie/set",
            get(wpt_xhr_credentials_request_cookie_set),
        )
        .route(
            "/wpt/runtime/xhr/credentials/request-cookie/check",
            get(wpt_xhr_credentials_request_cookie_check),
        )
        .route(
            "/wpt/runtime/xhr/credentials/default-response-cookie/set",
            get(wpt_xhr_credentials_default_response_cookie_set),
        )
        .route(
            "/wpt/runtime/xhr/credentials/default-response-cookie/check",
            get(wpt_xhr_credentials_default_response_cookie_check),
        )
        .route(
            "/wpt/runtime/xhr/credentials/include-response-cookie/set",
            get(wpt_xhr_credentials_include_response_cookie_set),
        )
        .route(
            "/wpt/runtime/xhr/credentials/include-response-cookie/check",
            get(wpt_xhr_credentials_include_response_cookie_check),
        )
        .route("/common/redirect.py", get(wpt_common_redirect))
        .route(
            "/service-workers/service-worker/resources/redirect.py",
            get(wpt_service_worker_redirect),
        )
        .route("/common/blank.html", get(wpt_common_blank_html))
        .route("/common/{*path}", get(wpt_root_common_asset))
        .route("/resources/{*path}", get(wpt_root_resources_asset))
        .route("/workers/{*path}", get(wpt_root_workers_asset))
        .route("/wpt/{*path}", get(wpt_fixture_asset))
        .with_state(runtime_state)
}

fn wpt_fixture_url(addr: std::net::SocketAddr, local_path: &str) -> String {
    wpt_fixture_url_for_origin(addr, local_path, WptManifestOrigin::Trusted)
}

fn wpt_fixture_url_for_origin(
    addr: std::net::SocketAddr,
    local_path: &str,
    origin: WptManifestOrigin,
) -> String {
    let host = match origin {
        WptManifestOrigin::Trusted => WPT_BROWSER_HOST,
        WptManifestOrigin::Insecure => WPT_INSECURE_HOST,
    };
    format!(
        "http://{}:{}/wpt/{}",
        host,
        addr.port(),
        local_path.trim_start_matches('/')
    )
}

fn wpt_case_url(
    addr: std::net::SocketAddr,
    local_path: &str,
    global: &str,
    query: &str,
    origin: WptManifestOrigin,
) -> String {
    let mut url = wpt_fixture_url_for_origin(addr, local_path, origin);
    append_raw_query(&mut url, query);
    if global == "worker" {
        append_raw_query(&mut url, &format!("{WPT_WORKER_WRAPPER_QUERY}=1"));
        url
    } else if global == "sharedworker" {
        append_raw_query(&mut url, &format!("{WPT_SHARED_WORKER_WRAPPER_QUERY}=1"));
        url
    } else if wpt_fixture_uses_window_wrapper(local_path) {
        append_raw_query(&mut url, &format!("{WPT_WINDOW_WRAPPER_QUERY}=1"));
        url
    } else {
        url
    }
}

fn append_raw_query(url: &mut String, raw_query: &str) {
    let raw_query = raw_query.trim().trim_start_matches('?');
    if raw_query.is_empty() {
        return;
    }
    if url.contains('?') {
        url.push('&');
    } else {
        url.push('?');
    }
    url.push_str(raw_query);
}

impl Drop for WptFixtureServer {
    fn drop(&mut self) {
        while let Some(shutdown_tx) = self.shutdown_txs.pop() {
            let _ = shutdown_tx.send(());
        }

        while let Some(task) = self.tasks.pop() {
            task.abort();
        }
    }
}

async fn wpt_fixture_asset(
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    State(runtime_state): State<WptFixtureRuntimeState>,
) -> Response {
    wpt_fixture_asset_response(&path, query, &runtime_state).await
}

async fn wpt_root_resources_asset(
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    State(runtime_state): State<WptFixtureRuntimeState>,
) -> Response {
    wpt_fixture_asset_response(&format!("resources/{path}"), query, &runtime_state).await
}

async fn wpt_root_common_asset(
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    State(runtime_state): State<WptFixtureRuntimeState>,
) -> Response {
    wpt_fixture_asset_response(&format!("upstream/common/{path}"), query, &runtime_state).await
}

async fn wpt_root_workers_asset(
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    State(runtime_state): State<WptFixtureRuntimeState>,
) -> Response {
    wpt_fixture_asset_response(&format!("upstream/workers/{path}"), query, &runtime_state).await
}

async fn wpt_common_redirect(Query(query): Query<HashMap<String, String>>) -> Response {
    match query.get("location") {
        Some(location) => Redirect::temporary(location).into_response(),
        None => (StatusCode::BAD_REQUEST, "missing redirect location").into_response(),
    }
}

async fn wpt_service_worker_redirect(Query(query): Query<HashMap<String, String>>) -> Response {
    wpt_service_worker_redirect_response(&query)
}

fn wpt_service_worker_redirect_response(query: &HashMap<String, String>) -> Response {
    let Some(location) = query.get("Redirect") else {
        return (StatusCode::BAD_REQUEST, "missing redirect target").into_response();
    };
    let status = match query.get("Status") {
        Some(raw_status) => match raw_status
            .parse::<u16>()
            .ok()
            .and_then(|status| StatusCode::from_u16(status).ok())
        {
            Some(status) => status,
            None => return (StatusCode::BAD_REQUEST, "invalid redirect status").into_response(),
        },
        None => StatusCode::FOUND,
    };
    let Ok(location) = HeaderValue::from_str(location) else {
        return (StatusCode::BAD_REQUEST, "invalid redirect target").into_response();
    };

    let mut response = Body::empty().into_response();
    *response.status_mut() = status;
    response.headers_mut().insert(LOCATION, location);
    if let Some(value) = query.get("ACAOrigin") {
        for item in value.split(',') {
            if let Ok(header_value) = HeaderValue::from_str(item) {
                response
                    .headers_mut()
                    .append(ACCESS_CONTROL_ALLOW_ORIGIN, header_value);
            }
        }
    }
    if let Some(value) = query.get("ACAHeaders")
        && let Ok(header_value) = HeaderValue::from_str(value)
    {
        response
            .headers_mut()
            .insert(ACCESS_CONTROL_ALLOW_HEADERS, header_value);
    }
    if let Some(value) = query.get("ACAMethods")
        && let Ok(header_value) = HeaderValue::from_str(value)
    {
        response
            .headers_mut()
            .insert(ACCESS_CONTROL_ALLOW_METHODS, header_value);
    }
    if let Some(value) = query.get("ACACredentials")
        && let Ok(header_value) = HeaderValue::from_str(value)
    {
        response
            .headers_mut()
            .insert(ACCESS_CONTROL_ALLOW_CREDENTIALS, header_value);
    }
    if let Some(value) = query.get("ACEHeaders")
        && let Ok(header_value) = HeaderValue::from_str(value)
    {
        response
            .headers_mut()
            .insert(ACCESS_CONTROL_EXPOSE_HEADERS, header_value);
    }
    response
}

async fn wpt_common_blank_html() -> Response {
    html_response("<!doctype html><title></title>".to_owned())
}

fn wpt_worker_location_helper_redirect() -> Response {
    Redirect::temporary("post-location-members.js?a").into_response()
}

fn wpt_worker_module_resource_redirect(query: &HashMap<String, String>) -> Response {
    let Some(location) = query.get("location") else {
        return (StatusCode::BAD_REQUEST, "missing redirect location").into_response();
    };
    Redirect::temporary(location).into_response()
}

fn wpt_worker_module_export_on_load_script() -> Response {
    let mut response = static_text_response(
        "export const importedModules = ['export-on-load-script.js'];".to_owned(),
        "text/javascript",
    );
    response
        .headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    response.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Service-Worker"),
    );
    response
}

fn wpt_worker_baseurl_redirect(fixture_path: &str) -> Option<Response> {
    let location = match fixture_path {
        "upstream/workers/baseurl/beta/import.py" => "../gamma/import.js",
        "upstream/workers/baseurl/beta/importScripts.py" => "../gamma/importScripts.js",
        "upstream/workers/baseurl/beta/sharedworker.py" => "../gamma/sharedworker.js",
        "upstream/workers/baseurl/beta/worker.py" => "../gamma/worker.js",
        "upstream/workers/baseurl/beta/xhr.py" => "../gamma/xhr.js",
        "upstream/workers/baseurl/beta/xhr-worker.py" => "../gamma/xhr-worker.js",
        "ported/sharedworker/resources/runtime-url/beta/importscripts.py" => {
            "../gamma/importscripts.js"
        }
        "ported/sharedworker/resources/runtime-url/beta/xhr-worker.py" => "../gamma/xhr-worker.js",
        _ => return None,
    };
    Some(Redirect::temporary(location).into_response())
}

async fn wpt_fixture_asset_response(
    path: &str,
    query: HashMap<String, String>,
    runtime_state: &WptFixtureRuntimeState,
) -> Response {
    let normalized_path = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    let source_path = wpt_worker_entry_source_path(&normalized_path);
    let is_worker_entry_request =
        source_path.is_some() || query.contains_key(WPT_WORKER_ENTRY_QUERY);
    let fixture_path = source_path.as_deref().unwrap_or(&normalized_path);

    let Some(fs_path) = sanitize_wpt_fixture_path(fixture_path) else {
        return (StatusCode::BAD_REQUEST, "invalid wpt fixture path").into_response();
    };

    if fixture_path == "upstream/workers/support/nosiniff-error-worker.py" {
        return wpt_worker_nosniff_error_response();
    }
    if fixture_path == "upstream/workers/support/imported_script.py" {
        return wpt_worker_imported_script_response(&query);
    }
    if fixture_path == "upstream/workers/interfaces/WorkerGlobalScope/location/helper-redirect.py" {
        return wpt_worker_location_helper_redirect();
    }
    if fixture_path == "upstream/workers/modules/resources/redirect.py" {
        return wpt_worker_module_resource_redirect(&query);
    }
    if fixture_path == "upstream/workers/modules/resources/export-on-load-script.py" {
        return wpt_worker_module_export_on_load_script();
    }
    if let Some(response) = wpt_worker_baseurl_redirect(fixture_path) {
        return response;
    }

    if is_worker_entry_request {
        if !wpt_fixture_uses_worker_wrapper(fixture_path) {
            return (
                StatusCode::BAD_REQUEST,
                "wpt worker entry wrapper only supports .any.js, .worker.js, and .sharedworker.js fixtures",
            )
                .into_response();
        }
        return match tokio::fs::read_to_string(&fs_path).await {
            Ok(source) => {
                let source = apply_wpt_substitutions(fixture_path, source, runtime_state, &query);
                javascript_response(wpt_worker_entry_script(fixture_path, &source, &query))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (StatusCode::NOT_FOUND, "wpt fixture not found").into_response()
            }
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to read wpt fixture",
            )
                .into_response(),
        };
    }

    if query.contains_key(WPT_WINDOW_WRAPPER_QUERY) {
        if !wpt_fixture_uses_window_wrapper(&normalized_path) {
            return (
                StatusCode::BAD_REQUEST,
                "wpt window wrapper only supports .any.js and .window.js fixtures",
            )
                .into_response();
        }
        return match tokio::fs::read_to_string(&fs_path).await {
            Ok(source) => {
                let source = apply_wpt_substitutions(fixture_path, source, runtime_state, &query);
                html_response(wpt_window_wrapper_html(&normalized_path, &source, &query))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (StatusCode::NOT_FOUND, "wpt fixture not found").into_response()
            }
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to read wpt fixture",
            )
                .into_response(),
        };
    }
    if query.contains_key(WPT_WORKER_WRAPPER_QUERY) {
        if !wpt_fixture_uses_worker_wrapper(&normalized_path) {
            return (
                StatusCode::BAD_REQUEST,
                "wpt worker wrapper only supports .any.js, .worker.js, and .sharedworker.js fixtures",
            )
                .into_response();
        }
        return match tokio::fs::read_to_string(&fs_path).await {
            Ok(source) => {
                let source = apply_wpt_substitutions(fixture_path, source, runtime_state, &query);
                html_response(wpt_worker_wrapper_html(&normalized_path, &source, &query))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (StatusCode::NOT_FOUND, "wpt fixture not found").into_response()
            }
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to read wpt fixture",
            )
                .into_response(),
        };
    }
    if query.contains_key(WPT_SHARED_WORKER_WRAPPER_QUERY) {
        if !wpt_fixture_uses_worker_wrapper(&normalized_path) {
            return (
                StatusCode::BAD_REQUEST,
                "wpt shared worker wrapper only supports .any.js, .worker.js, and .sharedworker.js fixtures",
            )
                .into_response();
        }
        return match tokio::fs::read_to_string(&fs_path).await {
            Ok(source) => {
                let source = apply_wpt_substitutions(fixture_path, source, runtime_state, &query);
                html_response(wpt_shared_worker_wrapper_html(
                    &normalized_path,
                    &source,
                    &query,
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (StatusCode::NOT_FOUND, "wpt fixture not found").into_response()
            }
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to read wpt fixture",
            )
                .into_response(),
        };
    }

    let Some(content_type) = wpt_fixture_content_type(&fs_path, fixture_path) else {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported wpt fixture type",
        )
            .into_response();
    };

    match read_static_wpt_fixture_body(&fs_path, fixture_path, content_type, runtime_state, &query)
        .await
    {
        Ok(response) => response,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (StatusCode::NOT_FOUND, "wpt fixture not found").into_response()
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to read wpt fixture",
        )
            .into_response(),
    }
}

async fn read_static_wpt_fixture_body(
    fs_path: &StdPath,
    fixture_path: &str,
    content_type: &'static str,
    runtime_state: &WptFixtureRuntimeState,
    query: &HashMap<String, String>,
) -> std::io::Result<Response> {
    if wpt_fixture_needs_text_processing(fixture_path, content_type) {
        let source = tokio::fs::read_to_string(fs_path).await?;
        let source = apply_wpt_substitutions(fixture_path, source, runtime_state, query);
        let mut response = static_text_response(
            adapt_upstream_html_fixture(fixture_path, source),
            content_type,
        );
        apply_wpt_headers_sidecar(&mut response, fs_path).await?;
        apply_wpt_pipe_headers(&mut response, query);
        return Ok(response);
    }

    let bytes = tokio::fs::read(fs_path).await?;
    let mut response = static_bytes_response(bytes, content_type);
    apply_wpt_headers_sidecar(&mut response, fs_path).await?;
    apply_wpt_pipe_headers(&mut response, query);
    Ok(response)
}

async fn apply_wpt_headers_sidecar(response: &mut Response, fs_path: &StdPath) -> io::Result<()> {
    let sidecar_path = PathBuf::from(format!("{}.headers", fs_path.display()));
    let source = match tokio::fs::read_to_string(&sidecar_path).await {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for (index, line) in source.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "invalid WPT .headers line {} in {}",
                    index + 1,
                    sidecar_path.display()
                ),
            ));
        };
        let name = HeaderName::from_bytes(name.trim().as_bytes()).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "invalid WPT .headers name on line {} in {}: {error}",
                    index + 1,
                    sidecar_path.display()
                ),
            )
        })?;
        let value = HeaderValue::from_str(value.trim()).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "invalid WPT .headers value on line {} in {}: {error}",
                    index + 1,
                    sidecar_path.display()
                ),
            )
        })?;
        response.headers_mut().append(name, value);
    }
    Ok(())
}

fn apply_wpt_pipe_headers(response: &mut Response, query: &HashMap<String, String>) {
    let Some(pipe) = query.get("pipe") else {
        return;
    };
    for command in pipe.split('|') {
        let command = command.trim();
        let Some(inner) = command
            .strip_prefix("header(")
            .and_then(|value| value.strip_suffix(')'))
        else {
            continue;
        };
        let Some((name, value)) = inner.split_once(',') else {
            continue;
        };
        let Ok(name) = HeaderName::from_bytes(name.trim().as_bytes()) else {
            continue;
        };
        let Ok(value) = HeaderValue::from_str(value.trim()) else {
            continue;
        };
        response.headers_mut().append(name, value);
    }
}

fn wpt_fixture_needs_text_processing(fixture_path: &str, content_type: &str) -> bool {
    content_type.starts_with("text/html") || wpt_fixture_uses_substitution(fixture_path)
}

fn wpt_worker_entry_source_path(path: &str) -> Option<String> {
    let path = path.trim_start_matches('/');
    if let Some(source) = path.strip_suffix(".any.worker.js") {
        return Some(format!("{source}.any.js"));
    }
    if let Some(source) = path.strip_suffix(".any.sharedworker.js") {
        return Some(format!("{source}.any.js"));
    }
    None
}

fn sanitize_wpt_fixture_path(path: &str) -> Option<PathBuf> {
    let mut fs_path = PathBuf::from(wpt_fixture_root());
    let candidate = StdPath::new(path);
    let mut saw_top_level_dir = false;
    for component in candidate.components() {
        match component {
            Component::Normal(segment) => {
                if !saw_top_level_dir {
                    let segment = segment.to_str()?;
                    if !WPT_STATIC_FIXTURE_TOP_LEVEL_DIRS.contains(&segment) {
                        return None;
                    }
                    saw_top_level_dir = true;
                }
                fs_path.push(segment);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if !saw_top_level_dir {
        return None;
    }
    Some(fs_path)
}

fn wpt_fixture_root() -> &'static StdPath {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        std::env::current_dir()
            .ok()
            .as_deref()
            .and_then(wpt_fixture_root_in_checkout)
            .unwrap_or_else(|| PathBuf::from(COMPILED_WPT_FIXTURE_ROOT))
    })
    .as_path()
}

fn wpt_fixture_root_in_checkout(start: &StdPath) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        for relative in [
            StdPath::new("moli-wpt-compat/fixtures/wpt"),
            StdPath::new("fixtures/wpt"),
        ] {
            let candidate = ancestor.join(relative);
            if candidate.join("manifest.toml").is_file()
                && candidate.join("expected.toml").is_file()
            {
                return Some(candidate);
            }
        }
    }
    None
}

fn wpt_fixture_uses_window_wrapper(path: &str) -> bool {
    let path = path.trim_start_matches('/');
    path.ends_with(".any.js") || path.ends_with(".window.js")
}

fn wpt_fixture_uses_worker_wrapper(path: &str) -> bool {
    let path = path.trim_start_matches('/');
    path.ends_with(".any.js") || path.ends_with(".worker.js") || path.ends_with(".sharedworker.js")
}

fn wpt_fixture_uses_substitution(path: &str) -> bool {
    StdPath::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|file_name| file_name.contains(".sub."))
}

fn wpt_fixture_content_type(path: &StdPath, fixture_path: &str) -> Option<&'static str> {
    if fixture_path == "upstream/workers/worker-url-encoding.html" {
        return Some("text/html");
    }
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("css") => Some("text/css; charset=utf-8"),
        Some("htm") => Some("text/html; charset=utf-8"),
        Some("html") => Some("text/html; charset=utf-8"),
        Some("js") => Some("application/javascript"),
        Some("json") => Some("application/json"),
        Some("txt") => Some("text/plain; charset=utf-8"),
        Some("xml") => Some("application/xml"),
        None if wpt_extensionless_worker_script(path) => Some("application/javascript"),
        _ => None,
    }
}

fn wpt_extensionless_worker_script(path: &StdPath) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !matches!(file_name, "1" | "Infinity" | "NaN" | "null" | "undefined") {
        return false;
    }
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    components.windows(4).any(|window| {
        window[0] == "workers"
            && window[1] == "constructors"
            && window[2] == "Worker"
            && window[3] == file_name
    }) || components.windows(4).any(|window| {
        window[0] == "workers"
            && window[1] == "constructors"
            && window[2] == "SharedWorker"
            && window[3] == file_name
    }) || components.windows(5).any(|window| {
        window[0] == "workers"
            && window[1] == "interfaces"
            && window[2] == "WorkerUtils"
            && window[3] == "importScripts"
            && window[4] == file_name
    })
}

fn apply_wpt_substitutions(
    path: &str,
    source: String,
    runtime_state: &WptFixtureRuntimeState,
    query: &HashMap<String, String>,
) -> String {
    if !wpt_fixture_uses_substitution(path) {
        return source;
    }

    let mut output = String::with_capacity(source.len());
    let mut rest = source.as_str();
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            output.push_str(&rest[start..]);
            return output;
        };

        let raw_token = &after_start[..end];
        if let Some(value) = wpt_substitution_value(raw_token.trim(), path, runtime_state, query) {
            output.push_str(&value);
        } else {
            output.push_str("{{");
            output.push_str(raw_token);
            output.push_str("}}");
        }
        rest = &after_start[end + 2..];
    }
    output.push_str(rest);
    output
}

fn wpt_substitution_value(
    token: &str,
    path: &str,
    runtime_state: &WptFixtureRuntimeState,
    query: &HashMap<String, String>,
) -> Option<String> {
    let primary_port = runtime_state.primary_addr.port().to_string();
    let secondary_port = runtime_state.secondary_addr.port().to_string();
    let primary_https_port = runtime_state.primary_https_addr.port().to_string();
    let secondary_https_port = runtime_state.secondary_https_addr.port().to_string();

    match token {
        "host" | "browser_host" => Some(WPT_BROWSER_HOST.to_owned()),
        "location[scheme]" => Some("http".to_owned()),
        "location[host]" => Some(wpt_fixture_host_port(runtime_state)),
        "location[hostname]" => Some(WPT_BROWSER_HOST.to_owned()),
        "location[origin]" | "location[server]" => {
            Some(format!("http://{}", wpt_fixture_host_port(runtime_state)))
        }
        "location[path]" => Some(format!("/wpt/{}", path.trim_start_matches('/'))),
        "location[port]" => Some(primary_port),
        "location[query]" => Some(String::new()),
        "ports[http][0]" => Some(primary_port),
        "ports[http][1]" => Some(secondary_port),
        "ports[https][0]" => Some(primary_https_port),
        "ports[https][1]" => Some(secondary_https_port),
        _ if token.starts_with("GET[") && token.ends_with(']') => token
            .strip_prefix("GET[")
            .and_then(|name| name.strip_suffix(']'))
            .map(|name| query.get(name).cloned().unwrap_or_default()),
        _ if token.starts_with("domains[") || token.starts_with("hosts[") => {
            if token.contains("nonexistent") {
                Some(format!("nonexistent.{WPT_BROWSER_HOST}"))
            } else if token.contains("www1") || token.contains("alt") {
                Some(WPT_REMOTE_HOST.to_owned())
            } else {
                Some(WPT_BROWSER_HOST.to_owned())
            }
        }
        _ => None,
    }
}

fn wpt_fixture_host_port(runtime_state: &WptFixtureRuntimeState) -> String {
    format!("{}:{}", WPT_BROWSER_HOST, runtime_state.primary_addr.port())
}

fn adapt_upstream_html_fixture(path: &str, source: String) -> String {
    if !path.starts_with("upstream/") || !(path.ends_with(".html") || path.ends_with(".htm")) {
        return source;
    }
    if source.contains("moli-wpt-adapter.js") {
        return source;
    }
    for marker in [
        r#"<script src="/resources/testharnessreport.js"></script>"#,
        r#"<script src='/resources/testharnessreport.js'></script>"#,
        r#"<script src=/resources/testharnessreport.js></script>"#,
    ] {
        if source.contains(marker) {
            return source.replace(
                marker,
                &format!(r#"{marker}<script src="/wpt/resources/moli-wpt-adapter.js"></script>"#),
            );
        }
    }
    source
}

fn wpt_window_wrapper_html(
    script_path: &str,
    script_source: &str,
    query: &HashMap<String, String>,
) -> String {
    let script_src =
        html_escape_double_quoted_attribute(&wpt_public_script_url_with_query(script_path, query));
    let meta_scripts = wpt_window_wrapper_meta_script_tags(script_path, script_source);
    format!(
        "<!doctype html><meta charset=\"utf-8\"><script src=\"/wpt/resources/testharness.js\"></script><script src=\"/wpt/resources/testharnessreport.js\"></script><script src=\"/wpt/resources/moli-wpt-adapter.js\"></script><body><div id=\"log\"></div>{meta_scripts}<script src=\"{script_src}\"></script>"
    )
}

fn wpt_worker_wrapper_html(
    script_path: &str,
    _script_source: &str,
    query: &HashMap<String, String>,
) -> String {
    let script_src =
        html_escape_double_quoted_attribute(&wpt_worker_entry_public_url(script_path, query));
    format!(
        "<!doctype html><meta charset=\"utf-8\"><script src=\"/wpt/resources/testharness.js\"></script><script src=\"/wpt/resources/testharnessreport.js\"></script><script src=\"/wpt/resources/moli-wpt-adapter.js\"></script><body><div id=\"log\"></div><script>fetch_tests_from_worker(new Worker(\"{script_src}\"));</script>"
    )
}

fn wpt_shared_worker_wrapper_html(
    script_path: &str,
    _script_source: &str,
    query: &HashMap<String, String>,
) -> String {
    let script_src = html_escape_double_quoted_attribute(&wpt_shared_worker_entry_public_url(
        script_path,
        query,
    ));
    format!(
        "<!doctype html><meta charset=\"utf-8\"><script src=\"/wpt/resources/testharness.js\"></script><script src=\"/wpt/resources/testharnessreport.js\"></script><script src=\"/wpt/resources/moli-wpt-adapter.js\"></script><body><div id=\"log\"></div><script>fetch_tests_from_worker(new SharedWorker(\"{script_src}\"));</script>"
    )
}

fn wpt_worker_entry_public_url(script_path: &str, query: &HashMap<String, String>) -> String {
    let path = script_path.trim_start_matches('/');
    let mut url = if let Some(worker_path) = path.strip_prefix("upstream/workers/")
        && let Some(stem) = worker_path.strip_suffix(".any.js")
    {
        format!("/workers/{stem}.any.worker.js")
    } else if let Some(worker_path) = path.strip_prefix("upstream/workers/")
        && path.ends_with(".worker.js")
    {
        format!("/workers/{worker_path}")
    } else {
        format!("/wpt/{path}?{WPT_WORKER_ENTRY_QUERY}=1")
    };
    append_forwarded_wpt_query(&mut url, query);
    url
}

fn wpt_shared_worker_entry_public_url(
    script_path: &str,
    query: &HashMap<String, String>,
) -> String {
    let path = script_path.trim_start_matches('/');
    let mut url = if let Some(worker_path) = path.strip_prefix("upstream/workers/")
        && let Some(stem) = worker_path.strip_suffix(".any.js")
    {
        format!("/workers/{stem}.any.worker.js")
    } else {
        format!("/wpt/{path}?{WPT_WORKER_ENTRY_QUERY}=1")
    };
    append_forwarded_wpt_query(&mut url, query);
    url
}

fn wpt_worker_entry_script(
    script_path: &str,
    script_source: &str,
    query: &HashMap<String, String>,
) -> String {
    let mut body = String::new();
    if !script_path.ends_with(".worker.js") {
        body.push_str("importScripts(\"/resources/testharness.js\");\n");
    }
    for reference in extract_wpt_meta_script_references(script_source) {
        if is_local_wpt_harness_meta_script(&reference) {
            let script_src = js_string_literal(&reference);
            body.push_str(&format!("importScripts({script_src});\n"));
            continue;
        }
        let Ok(Some(resolved)) =
            resolve_wpt_static_resource_reference(script_path, &reference, "upstream")
        else {
            continue;
        };
        let script_src = js_string_literal(&wpt_public_script_url(&resolved.path_with_suffix()));
        body.push_str(&format!("importScripts({script_src});\n"));
    }
    let entry_src = js_string_literal(&wpt_public_script_url_with_query(script_path, query));
    body.push_str(&format!("importScripts({entry_src});\n"));
    body
}

fn wpt_public_script_url(path: &str) -> String {
    let path = path.trim_start_matches('/');
    if let Some(worker_path) = path.strip_prefix("upstream/workers/") {
        format!("/workers/{worker_path}")
    } else {
        format!("/wpt/{path}")
    }
}

fn wpt_public_script_url_with_query(path: &str, query: &HashMap<String, String>) -> String {
    let mut url = wpt_public_script_url(path);
    append_forwarded_wpt_query(&mut url, query);
    url
}

fn append_forwarded_wpt_query(url: &mut String, query: &HashMap<String, String>) {
    let mut keys = query
        .keys()
        .filter(|key| should_forward_wpt_query_param(key))
        .collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        let Some(value) = query.get(key) else {
            continue;
        };
        if is_wpt_subset_variant_query_param(key) {
            append_raw_query(url, key);
            continue;
        }
        append_raw_query(url, &format!("{key}={value}"));
    }
}

fn should_forward_wpt_query_param(key: &str) -> bool {
    !matches!(
        key,
        WPT_WINDOW_WRAPPER_QUERY
            | WPT_WORKER_WRAPPER_QUERY
            | WPT_SHARED_WORKER_WRAPPER_QUERY
            | WPT_WORKER_ENTRY_QUERY
    )
}

fn is_wpt_subset_variant_query_param(key: &str) -> bool {
    let Some((start, end)) = key.split_once('-') else {
        return false;
    };
    !start.is_empty()
        && start.bytes().all(|byte| byte.is_ascii_digit())
        && (end == "last" || (!end.is_empty() && end.bytes().all(|byte| byte.is_ascii_digit())))
}

fn wpt_window_wrapper_meta_script_tags(script_path: &str, script_source: &str) -> String {
    let mut tags = String::new();
    for reference in extract_wpt_meta_script_references(script_source) {
        if is_local_wpt_harness_meta_script(&reference) {
            let script_src = html_escape_double_quoted_attribute(&reference);
            tags.push_str(&format!("<script src=\"{script_src}\"></script>"));
            continue;
        }
        let Ok(Some(resolved)) =
            resolve_wpt_static_resource_reference(script_path, &reference, "upstream")
        else {
            continue;
        };
        let script_src =
            html_escape_double_quoted_attribute(&format!("/wpt/{}", resolved.path_with_suffix()));
        tags.push_str(&format!("<script src=\"{script_src}\"></script>"));
    }
    tags
}

fn is_local_wpt_harness_meta_script(reference: &str) -> bool {
    matches!(
        reference,
        "/resources/WebIDLParser.js" | "/resources/idlharness.js"
    )
}

fn static_text_response(source: String, content_type: &'static str) -> Response {
    let mut response = source.into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

fn static_bytes_response(source: Vec<u8>, content_type: &'static str) -> Response {
    let mut response = Body::from(source).into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

fn html_response(source: String) -> Response {
    static_text_response(source, "text/html; charset=utf-8")
}

fn javascript_response(source: String) -> Response {
    static_text_response(source, "application/javascript")
}

fn wpt_worker_imported_script_response(query: &HashMap<String, String>) -> Response {
    let Some(mime) = query.get("mime") else {
        return (StatusCode::BAD_REQUEST, "missing mime query").into_response();
    };
    let Ok(content_type) = HeaderValue::from_str(mime) else {
        return (StatusCode::BAD_REQUEST, "invalid mime query").into_response();
    };
    let mut response = String::new().into_response();
    response.headers_mut().insert(CONTENT_TYPE, content_type);
    response
}

fn wpt_worker_nosniff_error_response() -> Response {
    let mut response = static_text_response(String::new(), "text/html");
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn wpt_cookie_http_set() -> Response {
    response_with_set_cookie(
        "http-cookie=set",
        "wpt_http_cookie=fixture; Path=/wpt/runtime/cookies; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_location_port_config(
    State(runtime_state): State<WptFixtureRuntimeState>,
    headers: HeaderMap,
) -> Response {
    port_config_response(runtime_state, headers, "__lmLocationPortConfig")
}

async fn wpt_fixture_server_config(
    State(runtime_state): State<WptFixtureRuntimeState>,
    headers: HeaderMap,
) -> Response {
    port_config_response(runtime_state, headers, "__lmFixtureServerConfig")
}

fn port_config_response(
    runtime_state: WptFixtureRuntimeState,
    headers: HeaderMap,
    global_name: &'static str,
) -> Response {
    let current_port = host_header_port(&headers).unwrap_or(runtime_state.primary_addr.port());
    let alternate_port = alternate_wpt_fixture_port(&runtime_state, Some(current_port));
    let current_port = serde_json::to_string(&current_port.to_string())
        .expect("current WPT fixture port should serialize");
    let alternate_port = serde_json::to_string(&alternate_port.to_string())
        .expect("alternate WPT fixture port should serialize");
    javascript_response(format!(
        "globalThis.{global_name} = Object.assign(globalThis.{global_name} || {{}}, {{ currentPort: {current_port}, alternatePort: {alternate_port} }});"
    ))
}

async fn wpt_websocket_echo(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(|socket| async move {
        handle_wpt_echo_websocket(socket).await;
    })
}

async fn wpt_websocket_cookie_set() -> Response {
    response_with_set_cookie(
        "websocket-cookie=set",
        "wpt_ws_cookie=handshake; Path=/wpt/runtime/websocket; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_websocket_cookie_echo(headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    let cookie = headers
        .get(COOKIE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    ws.on_upgrade(|mut socket| async move {
        let _ = socket.send(WebSocketMessage::Text(cookie.into())).await;
        let _ = socket
            .send(WebSocketMessage::Close(Some(
                axum::extract::ws::CloseFrame {
                    code: 1000,
                    reason: "cookie echoed".into(),
                },
            )))
            .await;
    })
}

async fn wpt_websocket_subprotocol(ws: WebSocketUpgrade) -> Response {
    ws.protocols(["superchat"])
        .on_upgrade(|mut socket| async move {
            let _ = socket
                .send(WebSocketMessage::Text("subprotocol-open".into()))
                .await;
            handle_wpt_echo_websocket(socket).await;
        })
}

async fn handle_wpt_echo_websocket(mut socket: WebSocket) {
    while let Some(Ok(message)) = socket.recv().await {
        match message {
            WebSocketMessage::Text(text) => {
                let _ = socket.send(WebSocketMessage::Text(text)).await;
            }
            WebSocketMessage::Binary(bytes) => {
                let _ = socket.send(WebSocketMessage::Binary(bytes)).await;
            }
            WebSocketMessage::Close(_frame) => {
                // recv already queued the Close reply; flush before releasing TCP.
                let _ = futures_util::SinkExt::flush(&mut socket).await;
                break;
            }
            WebSocketMessage::Ping(payload) => {
                let _ = socket.send(WebSocketMessage::Pong(payload)).await;
            }
            WebSocketMessage::Pong(_) => {}
        }
    }
}

async fn wpt_websocket_server_close(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(|mut socket| async move {
        let _ = socket
            .send(WebSocketMessage::Close(Some(
                axum::extract::ws::CloseFrame {
                    code: 3001,
                    reason: "server done".into(),
                },
            )))
            .await;
    })
}

async fn wpt_websocket_slow_handshake(ws: WebSocketUpgrade) -> Response {
    sleep(Duration::from_millis(250)).await;
    ws.on_upgrade(|socket| async move {
        handle_wpt_echo_websocket(socket).await;
    })
}

async fn wpt_websocket_plain_200() -> Response {
    text_response("not a websocket upgrade")
}

async fn wpt_websocket_response_cookie(ws: WebSocketUpgrade) -> Response {
    let mut response = ws.on_upgrade(|mut socket| async move {
        let _ = socket
            .send(WebSocketMessage::Close(Some(
                axum::extract::ws::CloseFrame {
                    code: 1000,
                    reason: "cookie stored".into(),
                },
            )))
            .await;
    });
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_static("ws_response_cookie=ok; Path=/; SameSite=Lax"),
    );
    response
}

fn host_header_port(headers: &HeaderMap) -> Option<u16> {
    headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|host| host.rsplit_once(':').map(|(_, port)| port))
        .and_then(|port| port.parse().ok())
}

fn alternate_wpt_fixture_port(
    runtime_state: &WptFixtureRuntimeState,
    current_port: Option<u16>,
) -> u16 {
    match current_port {
        Some(port) if port == runtime_state.primary_addr.port() => {
            runtime_state.secondary_addr.port()
        }
        Some(port) if port == runtime_state.secondary_addr.port() => {
            runtime_state.primary_addr.port()
        }
        _ => runtime_state.secondary_addr.port(),
    }
}

async fn wpt_cookie_http_check(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "wpt_http_cookie=fixture") {
        "http-cookie=seen"
    } else {
        "http-cookie=missing"
    };
    text_response(body)
}

async fn wpt_cookie_path_scope_set() -> Response {
    response_with_set_cookie(
        "path-cookie=set",
        "wpt_path_cookie=match; Path=/wpt/runtime/cookies/path-scope; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_cookie_path_scope_check(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "wpt_path_cookie=match") {
        "path-cookie=seen"
    } else {
        "path-cookie=missing"
    };
    text_response(body)
}

async fn wpt_cookie_path_scope_extra_check(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "wpt_path_cookie=match") {
        "path-cookie=seen"
    } else {
        "path-cookie=missing"
    };
    text_response(body)
}

async fn wpt_cookie_invalid_domain_set() -> Response {
    response_with_set_cookie(
        "invalid-domain=set",
        "wpt_bad_domain=1; Domain=example.com; Path=/wpt/runtime/cookies; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_cookie_invalid_domain_check(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "wpt_bad_domain=1") {
        "invalid-domain=seen"
    } else {
        "invalid-domain=missing"
    };
    text_response(body)
}

async fn wpt_cookie_replace_red() -> Response {
    response_with_set_cookie(
        "replace-cookie=red",
        "wpt_replace_cookie=red; Path=/wpt/runtime/cookies/replace; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_cookie_replace_blue() -> Response {
    response_with_set_cookie(
        "replace-cookie=blue",
        "wpt_replace_cookie=blue; Path=/wpt/runtime/cookies/replace; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_cookie_replace_check(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "wpt_replace_cookie=blue") {
        "replace-cookie=blue"
    } else if has_cookie(&headers, "wpt_replace_cookie=red") {
        "replace-cookie=red"
    } else {
        "replace-cookie=missing"
    };
    text_response(body)
}

async fn wpt_cookie_samesite_set() -> Response {
    let mut response = response_with_set_cookie(
        "samesite-cookie=set",
        "wpt_samesite_strict=ok; Path=/wpt/runtime/cookies/samesite; HttpOnly; SameSite=Strict",
    );
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_static(
            "wpt_samesite_lax=ok; Path=/wpt/runtime/cookies/samesite; HttpOnly; SameSite=Lax",
        ),
    );
    response
}

async fn wpt_cookie_samesite_check(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "wpt_samesite_strict=ok")
        && has_cookie(&headers, "wpt_samesite_lax=ok")
    {
        "samesite-cookie=strict-lax"
    } else {
        "samesite-cookie=missing"
    };
    text_response(body)
}

async fn wpt_cookie_redirect_chain_start() -> Response {
    redirect_with_cookies(
        "/wpt/runtime/cookies/redirect-chain/middle",
        &[
            "wpt_chain=start; Path=/wpt/runtime/cookies/redirect-chain; HttpOnly; SameSite=Lax",
            "wpt_common=one; Path=/wpt/runtime/cookies; HttpOnly; SameSite=Lax",
        ],
    )
}

async fn wpt_cookie_redirect_chain_middle(headers: HeaderMap) -> Response {
    if !has_cookie(&headers, "wpt_chain=start") || !has_cookie(&headers, "wpt_common=one") {
        return text_response("cookie-chain=broken");
    }

    redirect_with_cookies(
        "/wpt/runtime/cookies/redirect-chain/final",
        &[
            "wpt_common=two; Path=/wpt/runtime/cookies; HttpOnly; SameSite=Lax",
            "wpt_middle=seen; Path=/wpt/runtime/cookies/redirect-chain; HttpOnly; SameSite=Lax",
        ],
    )
}

async fn wpt_cookie_redirect_chain_final(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "wpt_chain=start")
        && has_cookie(&headers, "wpt_common=two")
        && has_cookie(&headers, "wpt_middle=seen")
    {
        "cookie-chain=ok"
    } else {
        "cookie-chain=broken"
    };
    text_response(body)
}

async fn wpt_xhr_text() -> Response {
    text_response(WPT_XHR_TEXT_BODY)
}

async fn wpt_xhr_html_text() -> Response {
    html_response(format!(
        "<!doctype html><html><body>{WPT_XHR_TEXT_BODY}</body></html>"
    ))
}

async fn wpt_xhr_json() -> Response {
    json_response(WPT_XHR_JSON_BODY)
}

async fn wpt_xhr_binary() -> Response {
    binary_response(&[0u8, 0, 1, 2, 0, 0, 9])
}

async fn wpt_xhr_redirect() -> Response {
    Redirect::to("/wpt/runtime/xhr/text").into_response()
}

async fn wpt_xhr_404() -> Response {
    (StatusCode::NOT_FOUND, "Not Found").into_response()
}

async fn wpt_xhr_500() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
}

async fn wpt_xhr_slow() -> Response {
    sleep(Duration::from_millis(75)).await;
    text_response(WPT_XHR_TEXT_BODY)
}

async fn wpt_xhr_very_slow() -> Response {
    sleep(Duration::from_millis(250)).await;
    text_response(WPT_XHR_TEXT_BODY)
}

async fn wpt_xhr_empty() -> Response {
    text_response("")
}

async fn wpt_xhr_abort_observing(
    State(runtime_state): State<WptFixtureRuntimeState>,
    Path(token): Path<String>,
) -> Response {
    abort_observing_response(Arc::clone(&runtime_state.abort_observations), token)
}

async fn wpt_xhr_abort_observing_status(
    State(runtime_state): State<WptFixtureRuntimeState>,
    Path(token): Path<String>,
) -> Response {
    abort_observing_status_response(&runtime_state.abort_observations, &token)
}

async fn wpt_fetch_abort_observing(
    State(runtime_state): State<WptFixtureRuntimeState>,
    Path(token): Path<String>,
) -> Response {
    abort_observing_response(Arc::clone(&runtime_state.abort_observations), token)
}

async fn wpt_fetch_abort_observing_status(
    State(runtime_state): State<WptFixtureRuntimeState>,
    Path(token): Path<String>,
) -> Response {
    abort_observing_status_response(&runtime_state.abort_observations, &token)
}

fn abort_observing_response(
    observations: Arc<AbortObservationRegistry>,
    token: String,
) -> Response {
    observations.mark_pending(&token);

    let observed_token = token;
    let (tx, rx) = mpsc::channel(2);

    tokio::spawn(async move {
        if tx
            .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from_static(
                b"partial-",
            )))
            .await
            .is_err()
        {
            observations.mark_complete(&observed_token, true);
            return;
        }
        let disconnected = tokio::select! {
            _ = tx.closed() => true,
            result = async {
                sleep(Duration::from_millis(1000)).await;
                tx.send(Ok::<Bytes, std::convert::Infallible>(Bytes::from_static(
                    b"tail",
                )))
                .await
            } => result.is_err(),
        };
        observations.mark_complete(&observed_token, disconnected);
    });

    Response::builder()
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from_stream(ReceiverStream::new(rx)))
        .expect("xhr abort observer response should build")
}

fn abort_observing_status_response(
    observations: &AbortObservationRegistry,
    token: &str,
) -> Response {
    let payload = match observations.lookup(token) {
        Some(Some(disconnected)) => serde_json::json!({
            "complete": true,
            "disconnected": disconnected,
        }),
        Some(None) => serde_json::json!({
            "complete": false,
            "disconnected": serde_json::Value::Null,
        }),
        None => serde_json::json!({
            "complete": false,
            "missing": true,
        }),
    };
    json_string_response(payload.to_string())
}

async fn wpt_xhr_request_headers(headers: HeaderMap) -> Response {
    let payload = serde_json::json!({
        "x_test": header_value(&headers, "x-test"),
        "accept": header_value(&headers, "accept"),
        "referer": header_value(&headers, "referer"),
        "sec_fetch_site": header_value(&headers, "sec-fetch-site"),
        "sec_fetch_mode": header_value(&headers, "sec-fetch-mode"),
        "sec_fetch_dest": header_value(&headers, "sec-fetch-dest"),
        "sec_ch_ua_present": header_value(&headers, "sec-ch-ua").is_some(),
        "sec_ch_ua_mobile": header_value(&headers, "sec-ch-ua-mobile"),
        "sec_ch_ua_platform": header_value(&headers, "sec-ch-ua-platform"),
    });
    json_string_response(payload.to_string())
}

async fn wpt_xhr_echo_body(headers: HeaderMap, body: Bytes) -> Response {
    let payload = serde_json::json!({
        "body": String::from_utf8_lossy(&body).to_string(),
        "content_type": header_value(&headers, "content-type"),
        "x_body_test": header_value(&headers, "x-body-test"),
    });
    json_string_response(payload.to_string())
}

async fn wpt_echo_body_hex(headers: HeaderMap, body: Bytes) -> Response {
    let payload = serde_json::json!({
        "body_hex": bytes_hex(&body),
        "content_type": header_value(&headers, "content-type"),
    });
    json_string_response(payload.to_string())
}

fn bytes_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

async fn wpt_cors_allow(headers: HeaderMap) -> Response {
    cors_text_response(&headers, "cors-ok")
}

async fn wpt_cors_deny() -> Response {
    text_response("cors-deny")
}

async fn wpt_cors_exposed_headers(headers: HeaderMap) -> Response {
    let mut response = cors_text_response(&headers, "cors-exposed");
    let response_headers = response.headers_mut();
    response_headers.insert("content-language", HeaderValue::from_static("en-US"));
    response_headers.insert("x-visible", HeaderValue::from_static("visible"));
    response_headers.insert("x-case-visible", HeaderValue::from_static("case-visible"));
    response_headers.insert("x-hidden", HeaderValue::from_static("hidden"));
    response_headers.insert(
        ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("X-Visible, X-Case-Visible"),
    );
    response
}

async fn wpt_cors_exposed_headers_wildcard(headers: HeaderMap) -> Response {
    let mut response = cors_text_response(&headers, "cors-exposed-wildcard");
    let response_headers = response.headers_mut();
    response_headers.insert("x-wildcard-visible", HeaderValue::from_static("wildcard"));
    response_headers.insert(ACCESS_CONTROL_EXPOSE_HEADERS, HeaderValue::from_static("*"));
    response
}

async fn wpt_cors_exposed_headers_wildcard_credentials(headers: HeaderMap) -> Response {
    let mut response = credentials_text_response(&headers, "cors-exposed-wildcard-credentials");
    let response_headers = response.headers_mut();
    response_headers.insert(
        "x-wildcard-credentialed",
        HeaderValue::from_static("credentialed"),
    );
    response_headers.insert(ACCESS_CONTROL_EXPOSE_HEADERS, HeaderValue::from_static("*"));
    response
}

async fn wpt_cors_preflight_allow(method: Method, headers: HeaderMap, body: Bytes) -> Response {
    cors_preflight_response(
        method,
        headers,
        body,
        "cors-preflight-allow",
        "POST, PUT",
        "content-type, x-preflight-test",
    )
}

async fn wpt_cors_preflight_deny_method(
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    cors_preflight_response(
        method,
        headers,
        body,
        "cors-preflight-deny-method",
        "GET",
        "content-type, x-preflight-test",
    )
}

async fn wpt_cors_preflight_deny_header(
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    cors_preflight_response(
        method,
        headers,
        body,
        "cors-preflight-deny-header",
        "POST",
        "content-type",
    )
}

async fn wpt_cors_preflight_redirect_to_allow(
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if method == Method::OPTIONS {
        return cors_redirect_response("/wpt/runtime/cors/preflight/allow", &headers);
    }
    cors_preflight_response(
        method,
        headers,
        body,
        "cors-preflight-redirect-to-allow",
        "POST",
        "content-type, x-preflight-test",
    )
}

async fn wpt_cors_preflight_redirect_to_preflight_required(
    State(runtime_state): State<WptFixtureRuntimeState>,
    Query(query): Query<HashMap<String, String>>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if method == Method::OPTIONS {
        return cors_preflight_response(
            method,
            headers,
            body,
            "cors-preflight-redirect-source",
            "POST",
            "content-type, x-preflight-test",
        );
    }
    let token = query.get("token").cloned().unwrap_or_default();
    let location = format!(
        "http://{}/wpt/runtime/cors/preflight/preflight-required-final?token={}",
        runtime_state.secondary_addr, token
    );
    cors_redirect_response(&location, &headers)
}

async fn wpt_cors_preflight_required_final(
    State(runtime_state): State<WptFixtureRuntimeState>,
    Query(query): Query<HashMap<String, String>>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let token = query.get("token").cloned().unwrap_or_default();
    if method == Method::OPTIONS {
        if !token.is_empty() {
            runtime_state
                .cors_preflight_observations
                .mark_preflight(&token);
        }
        return cors_preflight_response(
            method,
            headers,
            body,
            "cors-preflight-required-final",
            "POST",
            "content-type, x-preflight-test",
        );
    }

    let payload = serde_json::json!({
        "method": method.as_str(),
        "body": String::from_utf8_lossy(&body).to_string(),
        "x_preflight_test": header_value(&headers, "x-preflight-test"),
        "content_type": header_value(&headers, "content-type"),
        "preflighted": !token.is_empty() && runtime_state.cors_preflight_observations.take_preflighted(&token),
        "response": "cors-preflight-required-final",
    });
    let mut response = json_string_response(payload.to_string());
    add_cors_allow_origin_header(&mut response, &headers);
    response
}

async fn wpt_cors_redirect_to_credentials_allow(
    State(runtime_state): State<WptFixtureRuntimeState>,
    headers: HeaderMap,
) -> Response {
    let location = format!(
        "http://{}/wpt/runtime/cors/credentials/allow",
        runtime_state.secondary_addr
    );
    cors_redirect_response(&location, &headers)
}

async fn wpt_cors_redirect_to_same_origin_echo(
    State(runtime_state): State<WptFixtureRuntimeState>,
    headers: HeaderMap,
) -> Response {
    let location = format!(
        "http://{}:{}/wpt/runtime/cors/redirect/same-origin-echo",
        WPT_BROWSER_HOST,
        runtime_state.primary_addr.port()
    );
    cors_redirect_response(&location, &headers)
}

async fn wpt_cors_redirect_to_alt_to_same_origin_echo(
    State(runtime_state): State<WptFixtureRuntimeState>,
    headers: HeaderMap,
) -> Response {
    let location = format!(
        "http://{}:{}/wpt/runtime/cors/redirect/to-same-origin-echo",
        WPT_BROWSER_HOST,
        runtime_state.secondary_addr.port()
    );
    cors_redirect_response(&location, &headers)
}

async fn wpt_cors_redirect_to_cross_site_to_same_origin_echo(
    State(runtime_state): State<WptFixtureRuntimeState>,
    headers: HeaderMap,
) -> Response {
    let location = format!(
        "http://{}:{}/wpt/runtime/cors/redirect/to-same-origin-echo",
        WPT_REMOTE_HOST,
        runtime_state.secondary_addr.port()
    );
    cors_redirect_response(&location, &headers)
}

async fn wpt_cors_redirect_303_to_method_echo(
    State(runtime_state): State<WptFixtureRuntimeState>,
    headers: HeaderMap,
) -> Response {
    let location = format!(
        "http://{}/wpt/runtime/cors/redirect/method-echo",
        runtime_state.secondary_addr
    );
    let mut response = cors_redirect_response(&location, &headers);
    *response.status_mut() = StatusCode::SEE_OTHER;
    response
}

async fn wpt_cors_redirect_307_to_method_echo(
    State(runtime_state): State<WptFixtureRuntimeState>,
    headers: HeaderMap,
) -> Response {
    let location = format!(
        "http://{}/wpt/runtime/cors/redirect/method-echo",
        runtime_state.secondary_addr
    );
    let mut response = cors_redirect_response(&location, &headers);
    *response.status_mut() = StatusCode::TEMPORARY_REDIRECT;
    response
}

async fn wpt_cors_redirect_method_echo(
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let payload = serde_json::json!({
        "method": method.as_str(),
        "body": String::from_utf8_lossy(&body).to_string(),
        "content_type": header_value(&headers, "content-type"),
        "origin": header_value(&headers, ORIGIN.as_str()),
        "response": "cors-redirect-method-echo",
    });
    let mut response = json_string_response(payload.to_string());
    add_cors_allow_origin_header(&mut response, &headers);
    response
}

async fn wpt_cors_redirect_same_origin_echo(headers: HeaderMap) -> Response {
    let payload = serde_json::json!({
        "origin": header_value(&headers, ORIGIN.as_str()),
        "response": "cors-redirect-same-origin-echo",
    });
    // A return to the initiating URL still needs CORS authorization after a
    // cross-origin hop. Echo the actual Origin, including the literal null.
    let mut response = json_string_response(payload.to_string());
    add_cors_allow_origin_header(&mut response, &headers);
    response
}

async fn wpt_cors_credentials_allow(headers: HeaderMap) -> Response {
    credentials_text_response(&headers, "cors-credentials-ok")
}

async fn wpt_cors_credentials_wildcard() -> Response {
    let mut response = text_response("cors-credentials-wildcard");
    let headers = response.headers_mut();
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(
        ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    headers.insert(VARY, HeaderValue::from_static("Origin"));
    response
}

async fn wpt_fetch_credentials_request_cookie_set(headers: HeaderMap) -> Response {
    credentials_response_with_set_cookie(
        &headers,
        "request-cookie=set",
        "wpt_fetch_credentials_request=fixture; Path=/wpt/runtime/fetch/credentials; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_fetch_binary_response() -> Response {
    binary_response(&[0x00, 0xff, 0x41, 0x80, 0x0a, 0x0d])
}

async fn wpt_fetch_credentials_request_cookie_check(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "wpt_fetch_credentials_request=fixture") {
        "request-cookie=seen"
    } else {
        "request-cookie=missing"
    };
    credentials_text_response(&headers, body)
}

async fn wpt_fetch_credentials_default_response_cookie_set(headers: HeaderMap) -> Response {
    credentials_response_with_set_cookie(
        &headers,
        "default-response-cookie=set",
        "wpt_fetch_credentials_default_response=stored; Path=/wpt/runtime/fetch/credentials; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_fetch_credentials_default_response_cookie_check(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "wpt_fetch_credentials_default_response=stored") {
        "default-response-cookie=seen"
    } else {
        "default-response-cookie=missing"
    };
    credentials_text_response(&headers, body)
}

async fn wpt_fetch_credentials_include_response_cookie_set(headers: HeaderMap) -> Response {
    credentials_response_with_set_cookie(
        &headers,
        "include-response-cookie=set",
        "wpt_fetch_credentials_include_response=stored; Path=/wpt/runtime/fetch/credentials; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_fetch_credentials_include_response_cookie_check(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "wpt_fetch_credentials_include_response=stored") {
        "include-response-cookie=seen"
    } else {
        "include-response-cookie=missing"
    };
    credentials_text_response(&headers, body)
}

async fn wpt_fetch_credentials_omit_response_cookie_set(headers: HeaderMap) -> Response {
    credentials_response_with_set_cookie(
        &headers,
        "omit-response-cookie=set",
        "wpt_fetch_credentials_omit_response=stored; Path=/wpt/runtime/fetch/credentials; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_fetch_credentials_omit_response_cookie_check(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "wpt_fetch_credentials_omit_response=stored") {
        "omit-response-cookie=seen"
    } else {
        "omit-response-cookie=missing"
    };
    credentials_text_response(&headers, body)
}

async fn wpt_fetch_credentials_streaming_response_cookie_set(
    Path(mode): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some((body, cookie)) = streaming_fetch_credentials_cookie(&mode) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    streaming_credentials_response_with_set_cookie(&headers, body, cookie)
}

async fn wpt_fetch_credentials_streaming_response_cookie_check(
    Path(mode): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some((body, cookie)) = streaming_fetch_credentials_cookie(&mode) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let cookie_pair = cookie
        .split(';')
        .next()
        .expect("streaming credentials cookie should include a name/value pair");
    let response_body = if has_cookie(&headers, cookie_pair) {
        body.seen
    } else {
        body.missing
    };
    credentials_text_response(&headers, response_body)
}

async fn wpt_xhr_credentials_request_cookie_set(headers: HeaderMap) -> Response {
    credentials_response_with_set_cookie(
        &headers,
        "request-cookie=set",
        "wpt_xhr_credentials_request=fixture; Path=/wpt/runtime/xhr/credentials; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_xhr_credentials_request_cookie_check(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "wpt_xhr_credentials_request=fixture") {
        "request-cookie=seen"
    } else {
        "request-cookie=missing"
    };
    credentials_text_response(&headers, body)
}

async fn wpt_xhr_credentials_default_response_cookie_set(headers: HeaderMap) -> Response {
    credentials_response_with_set_cookie(
        &headers,
        "default-response-cookie=set",
        "wpt_xhr_credentials_default_response=stored; Path=/wpt/runtime/xhr/credentials; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_xhr_credentials_default_response_cookie_check(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "wpt_xhr_credentials_default_response=stored") {
        "default-response-cookie=seen"
    } else {
        "default-response-cookie=missing"
    };
    credentials_text_response(&headers, body)
}

async fn wpt_xhr_credentials_include_response_cookie_set(headers: HeaderMap) -> Response {
    credentials_response_with_set_cookie(
        &headers,
        "include-response-cookie=set",
        "wpt_xhr_credentials_include_response=stored; Path=/wpt/runtime/xhr/credentials; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_xhr_credentials_include_response_cookie_check(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "wpt_xhr_credentials_include_response=stored") {
        "include-response-cookie=seen"
    } else {
        "include-response-cookie=missing"
    };
    credentials_text_response(&headers, body)
}

struct StreamingCredentialBody {
    set_head: &'static str,
    set_tail: &'static str,
    seen: &'static str,
    missing: &'static str,
}

fn streaming_fetch_credentials_cookie(
    mode: &str,
) -> Option<(StreamingCredentialBody, &'static str)> {
    match mode {
        "default" => Some((
            StreamingCredentialBody {
                set_head: "streaming-default-response-cookie=head-",
                set_tail: "tail",
                seen: "streaming-default-response-cookie=seen",
                missing: "streaming-default-response-cookie=missing",
            },
            "wpt_fetch_credentials_streaming_default=stored; Path=/wpt/runtime/fetch/credentials; HttpOnly; SameSite=Lax",
        )),
        "include" => Some((
            StreamingCredentialBody {
                set_head: "streaming-include-response-cookie=head-",
                set_tail: "tail",
                seen: "streaming-include-response-cookie=seen",
                missing: "streaming-include-response-cookie=missing",
            },
            "wpt_fetch_credentials_streaming_include=stored; Path=/wpt/runtime/fetch/credentials; HttpOnly; SameSite=Lax",
        )),
        "omit" => Some((
            StreamingCredentialBody {
                set_head: "streaming-omit-response-cookie=head-",
                set_tail: "tail",
                seen: "streaming-omit-response-cookie=seen",
                missing: "streaming-omit-response-cookie=missing",
            },
            "wpt_fetch_credentials_streaming_omit=stored; Path=/wpt/runtime/fetch/credentials; HttpOnly; SameSite=Lax",
        )),
        _ => None,
    }
}

async fn wpt_form_submit_echo(
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let payload = serde_json::json!({
        "method": method.as_str(),
        "search": uri.query().map(|query| format!("?{query}")).unwrap_or_default(),
        "body": String::from_utf8_lossy(&body).to_string(),
        "content_type": header_value(&headers, "content-type"),
    });
    html_response(format!(
        "<!doctype html><body>{}</body>",
        html_escape_text(&payload.to_string())
    ))
}

async fn wpt_worker_slow_script() -> Response {
    sleep(Duration::from_millis(75)).await;
    javascript_response(WPT_WORKER_SLOW_BODY.to_string())
}

async fn wpt_worker_module_redirect_start() -> Redirect {
    Redirect::temporary("/wpt/ported/worker/resources/worker-module-http-redirect-entry.js")
}

async fn wpt_worker_module_redirect_fragment_start() -> Redirect {
    Redirect::temporary(
        "/wpt/ported/worker/resources/worker-module-http-redirect-entry.js#redirect-fragment",
    )
}

async fn wpt_worker_module_credentials_cookie_set(headers: HeaderMap) -> Response {
    credentials_response_with_set_cookie(
        &headers,
        "module-worker-cookie=set",
        "wpt_worker_module_credentials=fixture; Path=/wpt/runtime/worker/module-credentials; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_worker_module_credentials_cookie_check(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "wpt_worker_module_credentials=fixture") {
        "module-worker-cookie=seen"
    } else {
        "module-worker-cookie=missing"
    };
    credentials_text_response(&headers, body)
}

async fn wpt_worker_module_credentials_main(
    State(runtime_state): State<WptFixtureRuntimeState>,
) -> Response {
    let dependency_url = format!(
        "http://{}/wpt/runtime/worker/module-credentials/dependency.js",
        runtime_state.secondary_addr
    );
    javascript_response(format!(
        "import {{ credentialCookie }} from {};\npostMessage({{ credentialCookie, metaUrl: import.meta.url }});\n",
        serde_json::to_string(&dependency_url).expect("dependency URL should serialize")
    ))
}

async fn wpt_worker_module_credentials_dependency(headers: HeaderMap) -> Response {
    let credential_cookie = if has_cookie(&headers, "wpt_worker_module_credentials=fixture") {
        "seen"
    } else {
        "missing"
    };
    let mut response = javascript_response(format!(
        "export const credentialCookie = {};\n",
        serde_json::to_string(credential_cookie).expect("cookie status should serialize")
    ));
    add_cors_allow_origin_header(&mut response, &headers);
    response.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    response
}

async fn wpt_shared_worker_script_fetch_policy_cookie_set(headers: HeaderMap) -> Response {
    credentials_response_with_set_cookie(
        &headers,
        "shared-worker-script-cookie=set",
        "wpt_sharedworker_script_fetch_policy=fixture; Path=/wpt/runtime/sharedworker/script-fetch-policy; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_shared_worker_script_fetch_policy_remote_cookie_set(headers: HeaderMap) -> Response {
    credentials_response_with_set_cookie(
        &headers,
        "shared-worker-script-remote-cookie=set",
        "wpt_sharedworker_script_fetch_policy_remote=fixture; Path=/wpt/runtime/sharedworker/script-fetch-policy; HttpOnly; SameSite=Lax",
    )
}

async fn wpt_shared_worker_script_fetch_policy_report_cookie(headers: HeaderMap) -> Response {
    let credential_cookie = if has_cookie(&headers, "wpt_sharedworker_script_fetch_policy=fixture")
    {
        "seen"
    } else {
        "missing"
    };
    javascript_response(format!(
        "onconnect = (event) => {{ event.ports[0].postMessage({{ cookie: {} }}); }};\n",
        serde_json::to_string(credential_cookie).expect("cookie status should serialize")
    ))
}

async fn wpt_shared_worker_script_fetch_policy_report_import_cookie_main(
    State(runtime_state): State<WptFixtureRuntimeState>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let import_kind = query.get("kind").map(String::as_str).unwrap_or("static");
    let dependency_origin = query
        .get("dependencyOrigin")
        .map(String::as_str)
        .unwrap_or("same");
    let dependency_url = if let Some(dependency_url) = query.get("dependencyUrl") {
        dependency_url.clone()
    } else if dependency_origin == "remote" {
        format!(
            "http://{}/wpt/runtime/sharedworker/script-fetch-policy/report-import-cookie-remote-dependency.js",
            runtime_state.secondary_addr
        )
    } else {
        "/wpt/runtime/sharedworker/script-fetch-policy/report-import-cookie-dependency.js"
            .to_owned()
    };
    let serialized_dependency_url =
        serde_json::to_string(&dependency_url).expect("dependency URL should serialize");
    match import_kind {
        "dynamic" => javascript_response(format!(
            "onconnect = async (event) => {{ const module = await import({serialized_dependency_url}); event.ports[0].postMessage({{ cookie: module.credentialCookie }}); }};\n"
        )),
        _ => javascript_response(format!(
            "import {{ credentialCookie }} from {serialized_dependency_url};\nonconnect = (event) => {{ event.ports[0].postMessage({{ cookie: credentialCookie }}); }};\n"
        )),
    }
}

async fn wpt_shared_worker_script_fetch_policy_report_import_cookie_dependency(
    headers: HeaderMap,
) -> Response {
    shared_worker_script_fetch_policy_import_cookie_dependency_response(
        headers,
        "wpt_sharedworker_script_fetch_policy=fixture",
    )
}

async fn wpt_shared_worker_script_fetch_policy_report_import_cookie_remote_dependency(
    headers: HeaderMap,
) -> Response {
    shared_worker_script_fetch_policy_import_cookie_dependency_response(
        headers,
        "wpt_sharedworker_script_fetch_policy_remote=fixture",
    )
}

fn shared_worker_script_fetch_policy_import_cookie_dependency_response(
    headers: HeaderMap,
    expected_cookie: &str,
) -> Response {
    let credential_cookie = if has_cookie(&headers, expected_cookie) {
        "seen"
    } else {
        "missing"
    };
    let mut response = javascript_response(format!(
        "export const credentialCookie = {};\n",
        serde_json::to_string(credential_cookie).expect("cookie status should serialize")
    ));
    add_cors_allow_origin_header(&mut response, &headers);
    response.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    response
}

async fn wpt_shared_worker_script_fetch_policy_referrer_policy_main() -> Response {
    let mut response = javascript_response(
        r#"import { dependencyReferer } from "./report-referrer-dependency.js";
onconnect = (event) => {
  event.ports[0].postMessage({ dependencyReferer });
};
"#
        .to_owned(),
    );
    response.headers_mut().insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    response
}

async fn wpt_shared_worker_script_fetch_policy_report_referrer_dependency(
    headers: HeaderMap,
) -> Response {
    let referer = headers
        .get("referer")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    javascript_response(format!(
        "export const dependencyReferer = {};\n",
        serde_json::to_string(referer).expect("referer should serialize")
    ))
}

async fn wpt_shared_worker_script_fetch_policy_cross_origin_redirect_start(
    State(runtime_state): State<WptFixtureRuntimeState>,
) -> Redirect {
    Redirect::temporary(&format!(
        "http://{}/wpt/runtime/sharedworker/script-fetch-policy/cross-origin-target.js",
        runtime_state.secondary_addr
    ))
}

async fn wpt_shared_worker_script_fetch_policy_cross_origin_target() -> Response {
    javascript_response(
        "onconnect = (event) => event.ports[0].postMessage('executed-cross-origin');\n".to_owned(),
    )
}

async fn wpt_csp_reporting_target() -> Response {
    text_response("csp-reporting-target")
}

async fn wpt_csp_reporting_collect(
    State(runtime_state): State<WptFixtureRuntimeState>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(token) = query.get("token").filter(|token| !token.is_empty()) else {
        return (StatusCode::BAD_REQUEST, "missing report token").into_response();
    };
    runtime_state.csp_report_observations.push(
        token,
        CspReportObservation {
            content_type: header_value(&headers, CONTENT_TYPE.as_str()),
            body: String::from_utf8_lossy(&body).to_string(),
        },
    );
    StatusCode::NO_CONTENT.into_response()
}

async fn wpt_csp_reporting_take(
    State(runtime_state): State<WptFixtureRuntimeState>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let Some(token) = query.get("token").filter(|token| !token.is_empty()) else {
        return (StatusCode::BAD_REQUEST, "missing report token").into_response();
    };
    let reports = runtime_state
        .csp_report_observations
        .take(token)
        .into_iter()
        .map(|report| {
            serde_json::json!({
                "contentType": report.content_type,
                "body": report.body,
            })
        })
        .collect::<Vec<_>>();
    json_string_response(serde_json::json!({ "reports": reports }).to_string())
}

async fn wpt_worker_module_json_mime_config() -> Response {
    response_with_content_type(
        r#"{"name":"runtime-json","nested":{"value":11}}"#,
        "application/activity+json",
    )
}

async fn wpt_worker_module_json_mime_with_charset() -> Response {
    response_with_content_type(
        r#"{"name":"charset-json"}"#,
        "application/json; charset=utf-8",
    )
}

async fn wpt_worker_module_json_mime_javascript() -> Response {
    javascript_response("export default 'javascript-from-json-path';\n".to_owned())
}

async fn wpt_worker_module_json_mime_plain_json() -> Response {
    text_response(r#"{"name":"plain-json"}"#)
}

async fn wpt_worker_importscripts_cross_origin_target() -> Response {
    javascript_response(
        "globalThis.__lmCrossOriginImportScriptsLoaded = true;\nthrow new Error('cross-origin importScripts target should not execute');\n".to_string(),
    )
}

async fn wpt_worker_importscripts_redirect_to_cross_origin(
    State(runtime_state): State<WptFixtureRuntimeState>,
) -> Redirect {
    Redirect::temporary(&format!(
        "http://{}/wpt/runtime/worker/importscripts/cross-origin-target.js",
        runtime_state.secondary_addr
    ))
}

async fn wpt_worker_importscripts_args_worker() -> Response {
    javascript_response(WPT_WORKER_IMPORTSCRIPTS_ARGS_WORKER_BODY.to_string())
}

async fn wpt_worker_importscripts_undefined() -> Response {
    javascript_response(WPT_WORKER_IMPORTSCRIPTS_UNDEFINED_BODY.to_string())
}

async fn wpt_worker_importscripts_null() -> Response {
    javascript_response(WPT_WORKER_IMPORTSCRIPTS_NULL_BODY.to_string())
}

async fn wpt_worker_importscripts_one() -> Response {
    javascript_response(WPT_WORKER_IMPORTSCRIPTS_ONE_BODY.to_string())
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn html_escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn html_escape_double_quoted_attribute(value: &str) -> String {
    html_escape_text(value).replace('"', "&quot;")
}

fn has_cookie(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get(COOKIE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|cookies| {
            cookies
                .split(';')
                .map(str::trim)
                .any(|cookie| cookie == expected)
        })
}

fn response_with_set_cookie(body: &'static str, cookie: &'static str) -> Response {
    let mut response = text_response(body);
    response
        .headers_mut()
        .append(SET_COOKIE, HeaderValue::from_static(cookie));
    response
}

fn credentials_text_response(request_headers: &HeaderMap, body: &'static str) -> Response {
    let mut response = cors_text_response(request_headers, body);
    response.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    response
}

fn credentials_response_with_set_cookie(
    request_headers: &HeaderMap,
    body: &'static str,
    cookie: &'static str,
) -> Response {
    let mut response = response_with_set_cookie(body, cookie);
    add_cors_allow_origin_header(&mut response, request_headers);
    response.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    response
}

fn streaming_credentials_response_with_set_cookie(
    request_headers: &HeaderMap,
    body: StreamingCredentialBody,
    cookie: &'static str,
) -> Response {
    let (tx, rx) = mpsc::channel(2);
    tokio::spawn(async move {
        if tx
            .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from_static(
                body.set_head.as_bytes(),
            )))
            .await
            .is_err()
        {
            return;
        }
        sleep(Duration::from_millis(150)).await;
        let _ = tx
            .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from_static(
                body.set_tail.as_bytes(),
            )))
            .await;
    });

    let mut response = Response::builder()
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from_stream(ReceiverStream::new(rx)))
        .expect("streaming credentials response should build");
    add_cors_allow_origin_header(&mut response, request_headers);
    let headers = response.headers_mut();
    headers.insert(
        ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    headers.append(SET_COOKIE, HeaderValue::from_static(cookie));
    response
}

fn cors_text_response(request_headers: &HeaderMap, body: &'static str) -> Response {
    let mut response = text_response(body);
    add_cors_allow_origin_header(&mut response, request_headers);
    response
}

fn cors_preflight_response(
    method: Method,
    request_headers: HeaderMap,
    body: Bytes,
    response_body: &'static str,
    allowed_methods: &'static str,
    allowed_headers: &'static str,
) -> Response {
    if method == Method::OPTIONS {
        let payload = serde_json::json!({
            "access_control_request_method": header_value(&request_headers, ACCESS_CONTROL_REQUEST_METHOD.as_str()),
            "access_control_request_headers": header_value(&request_headers, ACCESS_CONTROL_REQUEST_HEADERS.as_str()),
        });
        let mut response = json_string_response(payload.to_string());
        add_cors_allow_origin_header(&mut response, &request_headers);
        let headers = response.headers_mut();
        headers.insert(
            ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static(allowed_methods),
        );
        headers.insert(
            ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static(allowed_headers),
        );
        return response;
    }

    let payload = serde_json::json!({
        "method": method.as_str(),
        "body": String::from_utf8_lossy(&body).to_string(),
        "x_preflight_test": header_value(&request_headers, "x-preflight-test"),
        "content_type": header_value(&request_headers, "content-type"),
        "response": response_body,
    });
    let mut response = json_string_response(payload.to_string());
    add_cors_allow_origin_header(&mut response, &request_headers);
    response
}

fn cors_redirect_response(location: &str, request_headers: &HeaderMap) -> Response {
    let mut response = Redirect::temporary(location).into_response();
    add_cors_allow_origin_header(&mut response, request_headers);
    response
}

fn add_cors_allow_origin_header(response: &mut Response, request_headers: &HeaderMap) {
    let allow_origin = request_headers
        .get(ORIGIN)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("*"));
    let headers = response.headers_mut();
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, allow_origin);
    headers.insert(VARY, HeaderValue::from_static("Origin"));
}

fn redirect_with_cookies(location: &'static str, cookies: &[&'static str]) -> Response {
    let mut response = Redirect::temporary(location).into_response();
    for cookie in cookies {
        response
            .headers_mut()
            .append(SET_COOKIE, HeaderValue::from_static(cookie));
    }
    response
}

fn text_response(source: &'static str) -> Response {
    let mut response = source.into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

fn response_with_content_type(source: &'static str, content_type: &'static str) -> Response {
    let mut response = source.into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

fn json_response(source: &'static str) -> Response {
    let mut response = source.into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

fn json_string_response(source: String) -> Response {
    let mut response = source.into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

fn js_string_literal(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization should not fail")
}

fn binary_response(bytes: &'static [u8]) -> Response {
    let mut response = bytes.to_vec().into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_wpt_fixture_path_rejects_escape_components() {
        assert!(sanitize_wpt_fixture_path("../manifest.toml").is_none());
        assert!(sanitize_wpt_fixture_path("/manifest.toml").is_none());
        assert!(sanitize_wpt_fixture_path("manifest.toml").is_none());
        assert!(sanitize_wpt_fixture_path("runtime/xhr/text").is_none());
        assert!(sanitize_wpt_fixture_path("").is_none());
    }

    #[test]
    fn sanitize_wpt_fixture_path_accepts_nested_fixture_path() {
        let path = sanitize_wpt_fixture_path("ported/dom/eventtarget-basic.html")
            .expect("nested WPT fixture path should be accepted");
        assert!(path.ends_with("ported/dom/eventtarget-basic.html"));

        let path = sanitize_wpt_fixture_path("upstream/url/url-origin.any.js")
            .expect("upstream WPT fixture path should be accepted");
        assert!(path.ends_with("upstream/url/url-origin.any.js"));
    }

    #[test]
    fn fixture_root_resolves_from_the_active_checkout() {
        let current_dir = std::env::current_dir().expect("test current directory");
        let active_checkout_root = wpt_fixture_root_in_checkout(&current_dir)
            .expect("WPT fixtures should be discoverable from the test checkout");
        let resolved_root = wpt_fixture_root();

        assert_eq!(
            std::fs::canonicalize(resolved_root).expect("resolved fixture root"),
            std::fs::canonicalize(active_checkout_root).expect("active checkout fixture root"),
            "a test artifact reused across worktrees must serve fixtures from the checkout that runs it"
        );
    }

    #[test]
    fn wpt_fixture_url_prefixes_fixture_route() {
        let addr = "127.0.0.1:12345".parse().expect("valid socket addr");

        assert_eq!(
            wpt_fixture_url(addr, "ported/dom/eventtarget-basic.html"),
            "http://localhost:12345/wpt/ported/dom/eventtarget-basic.html"
        );
        assert_eq!(
            wpt_fixture_url(addr, "/resources/testharness.js"),
            "http://localhost:12345/wpt/resources/testharness.js"
        );
    }

    #[test]
    fn wpt_case_url_wraps_window_js_fixtures() {
        let addr = "127.0.0.1:12345".parse().expect("valid socket addr");

        assert_eq!(
            wpt_case_url(
                addr,
                "upstream/url/url-origin.any.js",
                "window",
                "",
                WptManifestOrigin::Trusted
            ),
            "http://localhost:12345/wpt/upstream/url/url-origin.any.js?moli-wpt-window-wrapper=1"
        );
        assert_eq!(
            wpt_case_url(
                addr,
                "upstream/dom/events/EventTarget.window.js",
                "window",
                "",
                WptManifestOrigin::Trusted
            ),
            "http://localhost:12345/wpt/upstream/dom/events/EventTarget.window.js?moli-wpt-window-wrapper=1"
        );
        assert_eq!(
            wpt_case_url(
                addr,
                "upstream/workers/importscripts_mime.any.js",
                "worker",
                "",
                WptManifestOrigin::Trusted
            ),
            "http://localhost:12345/wpt/upstream/workers/importscripts_mime.any.js?moli-wpt-worker-wrapper=1"
        );
        assert_eq!(
            wpt_case_url(
                addr,
                "upstream/workers/examples/onconnect.any.js",
                "sharedworker",
                "",
                WptManifestOrigin::Trusted
            ),
            "http://localhost:12345/wpt/upstream/workers/examples/onconnect.any.js?moli-wpt-shared-worker-wrapper=1"
        );
        assert_eq!(
            wpt_case_url(
                addr,
                "ported/dom/eventtarget-basic.html",
                "window",
                "",
                WptManifestOrigin::Trusted
            ),
            "http://localhost:12345/wpt/ported/dom/eventtarget-basic.html"
        );
        assert_eq!(
            wpt_case_url(
                addr,
                "upstream/WebCryptoAPI/derive_bits_keys/pbkdf2.https.any.js",
                "window",
                "1-1000",
                WptManifestOrigin::Trusted
            ),
            "http://localhost:12345/wpt/upstream/WebCryptoAPI/derive_bits_keys/pbkdf2.https.any.js?1-1000&moli-wpt-window-wrapper=1"
        );
        assert_eq!(
            wpt_case_url(
                addr,
                "upstream/WebCryptoAPI/historical.any.js",
                "worker",
                "",
                WptManifestOrigin::Insecure
            ),
            "http://0.0.0.0:12345/wpt/upstream/WebCryptoAPI/historical.any.js?moli-wpt-worker-wrapper=1"
        );
    }

    #[test]
    fn wpt_window_wrapper_loads_harness_then_script() {
        let html = wpt_window_wrapper_html("upstream/url/url-origin.any.js", "", &HashMap::new());
        assert!(html.contains(r#"<script src="/wpt/resources/testharness.js"></script>"#));
        assert!(html.contains(r#"<script src="/wpt/resources/moli-wpt-adapter.js"></script>"#));
        assert!(html.contains(r#"<body><div id="log"></div>"#));
        assert!(html.contains(r#"<script src="/wpt/upstream/url/url-origin.any.js"></script>"#));
    }

    #[test]
    fn wpt_worker_wrapper_loads_harness_and_fetches_worker_results() {
        let html = wpt_worker_wrapper_html(
            "upstream/workers/importscripts_mime.any.js",
            "",
            &HashMap::new(),
        );

        assert!(html.contains(r#"<script src="/wpt/resources/testharness.js"></script>"#));
        assert!(html.contains(r#"<script src="/wpt/resources/moli-wpt-adapter.js"></script>"#));
        assert!(html.contains("fetch_tests_from_worker(new Worker("));
        assert!(html.contains("/workers/importscripts_mime.any.worker.js"));
    }

    #[test]
    fn wpt_worker_wrapper_uses_public_workers_url_for_worker_js_entrypoint() {
        let html = wpt_worker_wrapper_html(
            "upstream/workers/examples/general.worker.js",
            "",
            &HashMap::new(),
        );

        assert!(html.contains("fetch_tests_from_worker(new Worker("));
        assert!(html.contains("/workers/examples/general.worker.js"));
        assert!(!html.contains("moli-wpt-worker-entry"));
    }

    #[test]
    fn wpt_shared_worker_wrapper_uses_sharedworker_global_entry_url() {
        let html = wpt_shared_worker_wrapper_html(
            "upstream/workers/examples/onconnect.any.js",
            "",
            &HashMap::new(),
        );

        assert!(html.contains("fetch_tests_from_worker(new SharedWorker("));
        assert!(html.contains("/workers/examples/onconnect.any.worker.js"));
    }

    #[test]
    fn wpt_worker_entry_source_path_maps_synthetic_any_worker_url() {
        assert_eq!(
            wpt_worker_entry_source_path("upstream/workers/Worker-location.sub.any.worker.js")
                .as_deref(),
            Some("upstream/workers/Worker-location.sub.any.js")
        );
        assert_eq!(
            wpt_worker_entry_source_path("upstream/workers/examples/onconnect.any.sharedworker.js")
                .as_deref(),
            Some("upstream/workers/examples/onconnect.any.js")
        );
        assert_eq!(
            wpt_worker_entry_source_path("upstream/workers/Worker-location.sub.any.js"),
            None
        );
    }

    #[test]
    fn wpt_worker_entry_imports_harness_meta_scripts_and_entrypoint() {
        let source = "// META: script=../support/helper.js\n// META: script=/common/gc.js?run=1\n// META: script=report-error-helper.js\n";
        let script = wpt_worker_entry_script(
            "upstream/workers/interfaces/WorkerUtils/importScripts/report-error-cross-origin.sub.any.js",
            source,
            &HashMap::new(),
        );

        assert!(script.contains(r#"importScripts("/resources/testharness.js");"#));
        assert!(
            script
                .contains(r#"importScripts("/workers/interfaces/WorkerUtils/support/helper.js");"#)
        );
        assert!(script.contains(r#"importScripts("/wpt/upstream/common/gc.js?run=1");"#));
        assert!(
            script.contains(r#"importScripts("/workers/interfaces/WorkerUtils/importScripts/report-error-helper.js");"#)
        );
        assert!(script.contains(
            r#"importScripts("/workers/interfaces/WorkerUtils/importScripts/report-error-cross-origin.sub.any.js");"#
        ));
    }

    #[test]
    fn wpt_public_script_url_uses_root_worker_alias_for_upstream_workers() {
        assert_eq!(
            wpt_public_script_url(
                "upstream/workers/interfaces/WorkerUtils/importScripts/report-error-helper.js"
            ),
            "/workers/interfaces/WorkerUtils/importScripts/report-error-helper.js"
        );
        assert_eq!(
            wpt_public_script_url("upstream/common/gc.js?run=1"),
            "/wpt/upstream/common/gc.js?run=1"
        );
    }

    #[test]
    fn wpt_window_wrapper_escapes_script_path_attribute() {
        let html = wpt_window_wrapper_html("upstream/url/quote\"<&.any.js", "", &HashMap::new());
        assert!(html.contains("quote&quot;&lt;&amp;.any.js"));
    }

    #[test]
    fn wpt_worker_wrapper_forwards_variant_query_to_worker_entry() {
        let query = HashMap::from([
            ("1-1000".to_owned(), "1".to_owned()),
            (WPT_WORKER_WRAPPER_QUERY.to_owned(), "1".to_owned()),
        ]);
        let html = wpt_worker_wrapper_html(
            "upstream/WebCryptoAPI/derive_bits_keys/pbkdf2.https.any.js",
            "",
            &query,
        );

        assert!(html.contains("/wpt/upstream/WebCryptoAPI/derive_bits_keys/pbkdf2.https.any.js?moli-wpt-worker-entry=1&amp;1-1000"));
    }

    #[test]
    fn wpt_worker_entry_forwards_variant_query_to_test_script() {
        let query = HashMap::from([
            ("1-1000".to_owned(), "1".to_owned()),
            (WPT_WORKER_ENTRY_QUERY.to_owned(), "1".to_owned()),
        ]);
        let script = wpt_worker_entry_script(
            "upstream/WebCryptoAPI/derive_bits_keys/pbkdf2.https.any.js",
            "",
            &query,
        );

        assert!(script.contains(
            r#"importScripts("/wpt/upstream/WebCryptoAPI/derive_bits_keys/pbkdf2.https.any.js?1-1000");"#
        ));
    }

    #[test]
    fn upstream_html_fixture_injects_moli_adapter_after_harness_report() {
        let html = adapt_upstream_html_fixture(
            "upstream/FileAPI/file/File-constructor-endings.html",
            r#"<script src="/resources/testharness.js"></script><script src="/resources/testharnessreport.js"></script>"#.to_owned(),
        );

        assert!(
            html.contains(
                r#"<script src="/resources/testharnessreport.js"></script><script src="/wpt/resources/moli-wpt-adapter.js"></script>"#
            )
        );
    }

    #[test]
    fn upstream_html_fixture_injects_adapter_after_unquoted_harness_report() {
        let html = adapt_upstream_html_fixture(
            "upstream/workers/interfaces/DedicatedWorkerGlobalScope/postMessage/message-event.html",
            r#"<script src=/resources/testharness.js></script><script src=/resources/testharnessreport.js></script>"#.to_owned(),
        );

        assert!(
            html.contains(
                r#"<script src=/resources/testharnessreport.js></script><script src="/wpt/resources/moli-wpt-adapter.js"></script>"#
            )
        );
    }

    #[test]
    fn ported_html_fixture_does_not_inject_moli_adapter() {
        let source = r#"<script src="/resources/testharnessreport.js"></script>"#.to_owned();

        assert_eq!(
            adapt_upstream_html_fixture("ported/dom/eventtarget-basic.html", source.clone()),
            source
        );
    }

    #[test]
    fn wpt_window_wrapper_loads_meta_scripts_before_entrypoint() {
        let html = wpt_window_wrapper_html(
            "upstream/FileAPI/blob/Blob-constructor.any.js",
            "// META: script=../support/Blob.js\n// META: script=/common/gc.js?run=1\n",
            &HashMap::new(),
        );

        let helper = html
            .find(r#"<script src="/wpt/upstream/FileAPI/support/Blob.js"></script>"#)
            .expect("relative META script should be included");
        let root_helper = html
            .find(r#"<script src="/wpt/upstream/common/gc.js?run=1"></script>"#)
            .expect("root-relative META script should stay under upstream fixture root");
        let entrypoint = html
            .find(r#"<script src="/wpt/upstream/FileAPI/blob/Blob-constructor.any.js"></script>"#)
            .expect("entrypoint should be included");
        assert!(helper < entrypoint);
        assert!(root_helper < entrypoint);
    }

    #[test]
    fn wpt_fixture_content_type_accepts_text_fixture_types() {
        assert_eq!(
            wpt_fixture_content_type(
                StdPath::new("upstream/url/basic.window.js"),
                "upstream/url/basic.window.js",
            ),
            Some("application/javascript")
        );
        assert_eq!(
            wpt_fixture_content_type(
                StdPath::new("upstream/xhr/basic.sub.htm"),
                "upstream/xhr/basic.sub.htm",
            ),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(
            wpt_fixture_content_type(
                StdPath::new("upstream/resources/helper.txt"),
                "upstream/resources/helper.txt",
            ),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(
            wpt_fixture_content_type(
                StdPath::new("resources/testharness.css"),
                "resources/testharness.css",
            ),
            Some("text/css; charset=utf-8")
        );
        assert_eq!(
            wpt_fixture_content_type(
                StdPath::new("upstream/xhr/resources/well-formed.xml"),
                "upstream/xhr/resources/well-formed.xml",
            ),
            Some("application/xml")
        );
        assert_eq!(
            wpt_fixture_content_type(
                StdPath::new("upstream/workers/constructors/Worker/undefined"),
                "upstream/workers/constructors/Worker/undefined",
            ),
            Some("application/javascript")
        );
        assert_eq!(
            wpt_fixture_content_type(
                StdPath::new("upstream/workers/interfaces/WorkerUtils/importScripts/null"),
                "upstream/workers/interfaces/WorkerUtils/importScripts/null",
            ),
            Some("application/javascript")
        );
        assert_eq!(
            wpt_fixture_content_type(
                StdPath::new("upstream/resources/undefined"),
                "upstream/resources/undefined",
            ),
            None
        );
        assert_eq!(
            wpt_fixture_content_type(
                StdPath::new("upstream/resources/handler.py"),
                "upstream/resources/handler.py",
            ),
            None
        );
    }

    #[tokio::test]
    async fn static_worker_script_fixtures_preserve_non_utf8_bytes() {
        let fixture_path = "upstream/workers/constructors/Worker/script-utf16be.js";
        let fs_path = sanitize_wpt_fixture_path(fixture_path).expect("fixture path");
        let runtime_state = WptFixtureRuntimeState {
            primary_addr: "127.0.0.1:8000".parse().expect("primary addr"),
            secondary_addr: "127.0.0.1:8001".parse().expect("secondary addr"),
            primary_https_addr: "127.0.0.1:8443".parse().expect("primary HTTPS addr"),
            secondary_https_addr: "127.0.0.1:9443".parse().expect("secondary HTTPS addr"),
            abort_observations: Arc::new(AbortObservationRegistry::default()),
            cors_preflight_observations: Arc::new(CorsPreflightObservationRegistry::default()),
            csp_report_observations: Arc::new(CspReportObservationRegistry::default()),
        };

        let response = read_static_wpt_fixture_body(
            &fs_path,
            fixture_path,
            "application/javascript",
            &runtime_state,
            &HashMap::new(),
        )
        .await
        .expect("fixture should be read as bytes");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");

        assert_eq!(&body[..4], &[0xfe, 0xff, 0x00, 0x2f]);
    }

    #[tokio::test]
    async fn static_fixture_applies_headers_sidecar() {
        let fixture_path = "upstream/workers/modules/resources/export-on-load-script.js";
        let fs_path = sanitize_wpt_fixture_path(fixture_path).expect("fixture path");
        let runtime_state = WptFixtureRuntimeState {
            primary_addr: "127.0.0.1:8000".parse().expect("primary addr"),
            secondary_addr: "127.0.0.1:8001".parse().expect("secondary addr"),
            primary_https_addr: "127.0.0.1:8443".parse().expect("primary HTTPS addr"),
            secondary_https_addr: "127.0.0.1:9443".parse().expect("secondary HTTPS addr"),
            abort_observations: Arc::new(AbortObservationRegistry::default()),
            cors_preflight_observations: Arc::new(CorsPreflightObservationRegistry::default()),
            csp_report_observations: Arc::new(CspReportObservationRegistry::default()),
        };

        let response = read_static_wpt_fixture_body(
            &fs_path,
            fixture_path,
            "application/javascript",
            &runtime_state,
            &HashMap::new(),
        )
        .await
        .expect("fixture should be read with sidecar headers");

        assert_eq!(
            response.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static("*"))
        );
    }

    #[tokio::test]
    async fn static_fixture_applies_header_pipe() {
        let fixture_path = "upstream/fetch/api/resources/top.txt";
        let fs_path = sanitize_wpt_fixture_path(fixture_path).expect("fixture path");
        let runtime_state = WptFixtureRuntimeState {
            primary_addr: "127.0.0.1:8000".parse().expect("primary addr"),
            secondary_addr: "127.0.0.1:8001".parse().expect("secondary addr"),
            primary_https_addr: "127.0.0.1:8443".parse().expect("primary HTTPS addr"),
            secondary_https_addr: "127.0.0.1:9443".parse().expect("secondary HTTPS addr"),
            abort_observations: Arc::new(AbortObservationRegistry::default()),
            cors_preflight_observations: Arc::new(CorsPreflightObservationRegistry::default()),
            csp_report_observations: Arc::new(CspReportObservationRegistry::default()),
        };
        let mut query = HashMap::new();
        query.insert(
            "pipe".to_owned(),
            "header(Access-Control-Allow-Origin, null)|header(Bad Header,ok)|header(X-Bad,bad\r\nInjected: x)".to_owned(),
        );

        let response = read_static_wpt_fixture_body(
            &fs_path,
            fixture_path,
            "text/plain; charset=utf-8",
            &runtime_state,
            &query,
        )
        .await
        .expect("fixture should be read with pipe headers");

        assert_eq!(
            response.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static("null"))
        );
        assert!(response.headers().get("x-bad").is_none());
    }

    #[test]
    fn worker_nosniff_error_fixture_response_matches_upstream_handler() {
        let response = wpt_worker_nosniff_error_response();

        assert_eq!(
            response.headers().get(CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/html"))
        );
        assert_eq!(
            response.headers().get("x-content-type-options"),
            Some(&HeaderValue::from_static("nosniff"))
        );
    }

    #[test]
    fn worker_imported_script_fixture_response_uses_requested_mime() {
        let mut query = HashMap::new();
        query.insert(
            "mime".to_owned(),
            "text/javascript; charset=utf-8".to_owned(),
        );
        let response = wpt_worker_imported_script_response(&query);

        assert_eq!(
            response.headers().get(CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/javascript; charset=utf-8"))
        );
    }

    #[test]
    fn worker_location_helper_redirect_matches_upstream_handler() {
        let response = wpt_worker_location_helper_redirect();

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            response.headers().get("location"),
            Some(&HeaderValue::from_static("post-location-members.js?a"))
        );
    }

    #[test]
    fn worker_module_resource_redirect_matches_upstream_handler() {
        let mut query = HashMap::new();
        query.insert(
            "location".to_owned(),
            "/workers/modules/resources/export-on-load-script.js".to_owned(),
        );

        let response = wpt_worker_module_resource_redirect(&query);

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            response.headers().get(LOCATION),
            Some(&HeaderValue::from_static(
                "/workers/modules/resources/export-on-load-script.js"
            ))
        );
    }

    #[tokio::test]
    async fn worker_module_export_on_load_script_matches_upstream_handler() {
        let response = wpt_worker_module_export_on_load_script();

        assert_eq!(
            response.headers().get(CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/javascript"))
        );
        assert_eq!(
            response.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static("*"))
        );
        assert_eq!(
            response.headers().get(ACCESS_CONTROL_ALLOW_HEADERS),
            Some(&HeaderValue::from_static("Service-Worker"))
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        assert_eq!(
            body,
            Bytes::from_static(b"export const importedModules = ['export-on-load-script.js'];")
        );
    }

    #[test]
    fn service_worker_redirect_response_matches_upstream_query_shape() {
        let mut query = HashMap::new();
        query.insert(
            "Redirect".to_owned(),
            "http://example.test/target.js".to_owned(),
        );
        query.insert("ACAOrigin".to_owned(), "*".to_owned());
        query.insert("Status".to_owned(), "302".to_owned());

        let response = wpt_service_worker_redirect_response(&query);

        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(
            response.headers().get(LOCATION),
            Some(&HeaderValue::from_static("http://example.test/target.js"))
        );
        assert_eq!(
            response.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static("*"))
        );
    }

    #[test]
    fn wpt_substitution_replaces_basic_fixture_tokens() {
        let runtime_state = WptFixtureRuntimeState {
            primary_addr: "127.0.0.1:12345"
                .parse()
                .expect("valid primary socket addr"),
            secondary_addr: "127.0.0.1:23456"
                .parse()
                .expect("valid secondary socket addr"),
            primary_https_addr: "127.0.0.1:34567"
                .parse()
                .expect("valid primary HTTPS socket addr"),
            secondary_https_addr: "127.0.0.1:45678"
                .parse()
                .expect("valid secondary HTTPS socket addr"),
            abort_observations: Arc::new(AbortObservationRegistry::default()),
            cors_preflight_observations: Arc::new(CorsPreflightObservationRegistry::default()),
            csp_report_observations: Arc::new(CspReportObservationRegistry::default()),
        };

        let substituted = apply_wpt_substitutions(
            "upstream/dom/events/EventListener-incumbent-global-1.sub.html",
            "http://{{host}}:{{ports[http][0]}} \
             http://{{domains[www1]}}:{{ports[http][1]}} \
             https://{{domains[www1]}}:{{ports[https][0]}} \
             {{location[scheme]}} {{location[host]}} {{location[path]}} \
             {{hosts[alt][]}} {{unknown}}"
                .to_owned(),
            &runtime_state,
            &HashMap::new(),
        );

        assert!(substituted.contains("http://localhost:12345"));
        assert!(substituted.contains("http://127.0.0.1:23456"));
        assert!(substituted.contains("https://127.0.0.1:34567"));
        assert!(substituted.contains("http localhost:12345"));
        assert!(
            substituted
                .contains("/wpt/upstream/dom/events/EventListener-incumbent-global-1.sub.html")
        );
        assert!(substituted.contains("{{unknown}}"));
    }

    #[test]
    fn wpt_substitution_replaces_get_query_tokens() {
        let runtime_state = WptFixtureRuntimeState {
            primary_addr: "127.0.0.1:12345"
                .parse()
                .expect("valid primary socket addr"),
            secondary_addr: "127.0.0.1:23456"
                .parse()
                .expect("valid secondary socket addr"),
            primary_https_addr: "127.0.0.1:34567"
                .parse()
                .expect("valid primary HTTPS socket addr"),
            secondary_https_addr: "127.0.0.1:45678"
                .parse()
                .expect("valid secondary HTTPS socket addr"),
            abort_observations: Arc::new(AbortObservationRegistry::default()),
            cors_preflight_observations: Arc::new(CorsPreflightObservationRegistry::default()),
            csp_report_observations: Arc::new(CspReportObservationRegistry::default()),
        };
        let query = HashMap::from([
            (
                "action".to_owned(),
                "target/form-action-url-target.html".to_owned(),
            ),
            ("empty".to_owned(), String::new()),
        ]);

        let substituted = apply_wpt_substitutions(
            "upstream/html/semantics/forms/the-form-element/resources/form-with-action.sub.html",
            r#"<form action="{{GET[action]}}" data-empty="{{GET[empty]}}" data-missing="{{GET[missing]}}">"#
                .to_owned(),
            &runtime_state,
            &query,
        );

        assert_eq!(
            substituted,
            r#"<form action="target/form-action-url-target.html" data-empty="" data-missing="">"#
        );
    }

    #[test]
    fn wpt_substitution_ignores_non_sub_fixtures() {
        let runtime_state = WptFixtureRuntimeState {
            primary_addr: "127.0.0.1:12345"
                .parse()
                .expect("valid primary socket addr"),
            secondary_addr: "127.0.0.1:23456"
                .parse()
                .expect("valid secondary socket addr"),
            primary_https_addr: "127.0.0.1:34567"
                .parse()
                .expect("valid primary HTTPS socket addr"),
            secondary_https_addr: "127.0.0.1:45678"
                .parse()
                .expect("valid secondary HTTPS socket addr"),
            abort_observations: Arc::new(AbortObservationRegistry::default()),
            cors_preflight_observations: Arc::new(CorsPreflightObservationRegistry::default()),
            csp_report_observations: Arc::new(CspReportObservationRegistry::default()),
        };
        let source = "http://{{host}}:{{ports[http][0]}}".to_owned();

        assert_eq!(
            apply_wpt_substitutions(
                "upstream/url/url-origin.any.js",
                source.clone(),
                &runtime_state,
                &HashMap::new()
            ),
            source
        );
    }

    #[test]
    fn has_cookie_requires_full_cookie_pair_match() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("not_wpt=one_extra; theme=dark"),
        );

        assert!(!has_cookie(&headers, "wpt=one"));
    }

    #[test]
    fn has_cookie_matches_trimmed_cookie_pair() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("theme=dark; wpt=one; session=ok"),
        );

        assert!(has_cookie(&headers, "wpt=one"));
    }

    #[test]
    fn host_header_port_extracts_numeric_port() {
        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_static("127.0.0.1:34567"));

        assert_eq!(host_header_port(&headers), Some(34567));
    }

    #[test]
    fn alternate_wpt_fixture_port_toggles_between_primary_and_secondary() {
        let runtime_state = WptFixtureRuntimeState {
            primary_addr: "127.0.0.1:12345"
                .parse()
                .expect("valid primary socket addr"),
            secondary_addr: "127.0.0.1:23456"
                .parse()
                .expect("valid secondary socket addr"),
            primary_https_addr: "127.0.0.1:34567"
                .parse()
                .expect("valid primary HTTPS socket addr"),
            secondary_https_addr: "127.0.0.1:45678"
                .parse()
                .expect("valid secondary HTTPS socket addr"),
            abort_observations: Arc::new(AbortObservationRegistry::default()),
            cors_preflight_observations: Arc::new(CorsPreflightObservationRegistry::default()),
            csp_report_observations: Arc::new(CspReportObservationRegistry::default()),
        };

        assert_eq!(
            alternate_wpt_fixture_port(&runtime_state, Some(12345)),
            23456
        );
        assert_eq!(
            alternate_wpt_fixture_port(&runtime_state, Some(23456)),
            12345
        );
        assert_eq!(alternate_wpt_fixture_port(&runtime_state, None), 23456);
    }
}
