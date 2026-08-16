use super::*;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

static NET_UPSTREAM_XHR_404_THEN_200_REQUESTS: AtomicUsize = AtomicUsize::new(0);
static CONCURRENT_SHARED_STATE_REQUESTS_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static PARSER_IMAGE_FETCH_POLICY_TOKEN_COUNTER: AtomicUsize = AtomicUsize::new(0);
static PARSER_IMAGE_FETCH_POLICY_ASSET_REQUESTS: OnceLock<Mutex<HashMap<String, usize>>> =
    OnceLock::new();
static PARSE_TIME_ASYNC_CHUNKED_TAIL_GATES: OnceLock<
    Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
> = OnceLock::new();
static RUNTIME_OWNED_ASYNC_CHUNKED_TAIL_GATES: OnceLock<
    Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
> = OnceLock::new();
static RUNTIME_OWNED_IN_ORDER_ERROR_AFTER_DCL_GATES: OnceLock<
    Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
> = OnceLock::new();

struct ConcurrentSharedStateRequestGuard;

impl ConcurrentSharedStateRequestGuard {
    fn enter() -> (Self, bool) {
        let previous = CONCURRENT_SHARED_STATE_REQUESTS_IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
        (Self, previous > 0)
    }
}

impl Drop for ConcurrentSharedStateRequestGuard {
    fn drop(&mut self) {
        CONCURRENT_SHARED_STATE_REQUESTS_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(super) fn next_parser_image_fetch_policy_token() -> String {
    let token = PARSER_IMAGE_FETCH_POLICY_TOKEN_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("parser-image-fetch-policy-{token}")
}

fn parser_image_fetch_policy_asset_requests() -> &'static Mutex<HashMap<String, usize>> {
    PARSER_IMAGE_FETCH_POLICY_ASSET_REQUESTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn record_parser_image_fetch_policy_asset_request(token: &str) {
    let mut requests = parser_image_fetch_policy_asset_requests().lock();
    *requests.entry(token.to_owned()).or_default() += 1;
}

pub(super) fn parser_image_fetch_policy_asset_request_count(token: &str) -> usize {
    parser_image_fetch_policy_asset_requests()
        .lock()
        .get(token)
        .copied()
        .unwrap_or(0)
}

fn request_host_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn parse_time_async_chunked_tail_gate(host_key: &str) -> Arc<tokio::sync::Notify> {
    let gates = PARSE_TIME_ASYNC_CHUNKED_TAIL_GATES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut gates = gates.lock();
    gates
        .entry(host_key.to_owned())
        .or_insert_with(|| Arc::new(tokio::sync::Notify::new()))
        .clone()
}

fn remove_parse_time_async_chunked_tail_gate(host_key: &str) {
    let Some(gates) = PARSE_TIME_ASYNC_CHUNKED_TAIL_GATES.get() else {
        return;
    };
    let mut gates = gates.lock();
    gates.remove(host_key);
}

fn notify_parse_time_async_chunked_tail_gate_if_present(host_key: &str) {
    let Some(gates) = PARSE_TIME_ASYNC_CHUNKED_TAIL_GATES.get() else {
        return;
    };
    let notify = {
        let gates = gates.lock();
        gates.get(host_key).cloned()
    };
    if let Some(notify) = notify {
        notify.notify_one();
    }
}

fn runtime_owned_async_chunked_tail_gate(host_key: &str) -> Arc<tokio::sync::Notify> {
    let gates = RUNTIME_OWNED_ASYNC_CHUNKED_TAIL_GATES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut gates = gates.lock();
    gates
        .entry(host_key.to_owned())
        .or_insert_with(|| Arc::new(tokio::sync::Notify::new()))
        .clone()
}

fn remove_runtime_owned_async_chunked_tail_gate(host_key: &str) {
    let Some(gates) = RUNTIME_OWNED_ASYNC_CHUNKED_TAIL_GATES.get() else {
        return;
    };
    gates.lock().remove(host_key);
}

fn notify_runtime_owned_async_chunked_tail_gate_if_present(host_key: &str) {
    let Some(gates) = RUNTIME_OWNED_ASYNC_CHUNKED_TAIL_GATES.get() else {
        return;
    };
    let notify = {
        let gates = gates.lock();
        gates.get(host_key).cloned()
    };
    if let Some(notify) = notify {
        notify.notify_one();
    }
}

fn runtime_owned_in_order_error_after_dcl_gate(host_key: &str) -> Arc<tokio::sync::Notify> {
    let gates =
        RUNTIME_OWNED_IN_ORDER_ERROR_AFTER_DCL_GATES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut gates = gates.lock();
    gates
        .entry(host_key.to_owned())
        .or_insert_with(|| Arc::new(tokio::sync::Notify::new()))
        .clone()
}

fn remove_runtime_owned_in_order_error_after_dcl_gate(host_key: &str) {
    let Some(gates) = RUNTIME_OWNED_IN_ORDER_ERROR_AFTER_DCL_GATES.get() else {
        return;
    };
    gates.lock().remove(host_key);
}

pub(crate) fn notify_runtime_owned_in_order_error_after_dcl_gate(host_key: &str) {
    runtime_owned_in_order_error_after_dcl_gate(host_key).notify_one();
}

pub(super) async fn static_page() -> Html<&'static str> {
    Html(STATIC_HTML)
}

pub(super) async fn future_interval_done_page() -> Html<&'static str> {
    Html(FUTURE_INTERVAL_DONE_HTML)
}

pub(super) async fn encoding_gbk_meta_page() -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    tokio::spawn(async move {
        let chunks = vec![
            b"<!doctype html><html><head><meta charset=\"gbk\"><title>GBK</title></head><body data-charset=\"\"><main id=\"gbk\">".to_vec(),
            vec![0xCC],
            vec![
                0xAB, 0xC6, 0xBD, 0xD1, 0xF3, 0xBC, 0xD2, 0xBE, 0xD3, b' ', b'G', b'B',
                b'K', b' ', b'O', b'K',
            ],
            b"</main><script>document.body.setAttribute('data-charset', document.characterSet);</script></body></html>".to_vec(),
        ];
        for chunk in chunks {
            if tx
                .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from(chunk)))
                .await
                .is_err()
            {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    });

    Response::builder()
        .header(CONTENT_TYPE, "text/html")
        .body(Body::from_stream(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .expect("GBK streaming html response should build")
}

pub(super) async fn encoding_shift_jis_classic_script_page() -> Response {
    Response::builder()
        .header(CONTENT_TYPE, "text/html")
        .body(Body::from(
            r#"<!doctype html><html><head><meta charset="shift_jis"><title>Shift_JIS</title></head><body><main id="script-text"></main><script src="/encoding/shift-jis-classic-script.js"></script></body></html>"#,
        ))
        .expect("Shift_JIS classic script fixture page should build")
}

pub(super) async fn encoding_shift_jis_classic_script() -> Response {
    let mut body = b"document.getElementById(\"script-text\").textContent = \"".to_vec();
    body.extend_from_slice(&[0x96, 0xDA, 0x8E, 0x9F]);
    body.extend_from_slice(b"\";");
    Response::builder()
        .header(CONTENT_TYPE, "application/javascript")
        .body(Body::from(body))
        .expect("Shift_JIS classic script fixture should build")
}

pub(super) async fn encoding_child_shift_jis_classic_script_parent_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html><html><head><script>
window.addEventListener("message", event => {
  if (event.data && event.data.type === "child-shift-jis-script") {
    document.body.setAttribute("data-child-script-text", event.data.text);
  }
});
</script></head><body><iframe src="/encoding/child-shift-jis-classic-script"></iframe></body></html>"#,
    )
}

pub(super) async fn encoding_child_shift_jis_classic_script_page() -> Response {
    Response::builder()
        .header(CONTENT_TYPE, "text/html")
        .body(Body::from(
            r#"<!doctype html><html><head><meta charset="shift_jis"></head><body><script src="/encoding/child-shift-jis-classic-script.js"></script></body></html>"#,
        ))
        .expect("child Shift_JIS classic script page should build")
}

pub(super) async fn encoding_child_shift_jis_classic_script() -> Response {
    let mut body = b"document.body.setAttribute(\"data-script-text\", \"".to_vec();
    body.extend_from_slice(&[0x96, 0xDA, 0x8E, 0x9F]);
    body.extend_from_slice(
        b"\");parent.postMessage({ type: \"child-shift-jis-script\", text: document.body.getAttribute(\"data-script-text\") }, \"*\");",
    );
    Response::builder()
        .header(CONTENT_TYPE, "application/javascript")
        .body(Body::from(body))
        .expect("child Shift_JIS classic script fixture should build")
}

pub(super) async fn encoding_child_shift_jis_document_parent_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html><html><head><script>
window.addEventListener("message", event => {
  if (event.data && event.data.type === "child-shift-jis-document") {
    document.body.setAttribute("data-child-document-text", event.data.text);
    document.body.setAttribute("data-child-document-charset", event.data.characterSet);
    document.body.setAttribute("data-child-window-document-charset", event.data.windowDocumentCharacterSet);
    const frame = document.querySelector("iframe");
    document.body.setAttribute("data-child-content-document-charset", frame.contentDocument.characterSet);
  }
});
</script></head><body><iframe src="/encoding/child-shift-jis-document"></iframe></body></html>"#,
    )
}

pub(super) async fn encoding_child_shift_jis_document_page() -> Response {
    let mut body =
        b"<!doctype html><html><head><meta charset=\"shift_jis\"></head><body><main id=\"text\">"
            .to_vec();
    body.extend_from_slice(&[0x96, 0xDA, 0x8E, 0x9F]);
    body.extend_from_slice(
        br#"</main><script>
parent.postMessage({
  type: "child-shift-jis-document",
  text: document.getElementById("text").textContent,
  characterSet: document.characterSet,
  windowDocumentCharacterSet: window.document.characterSet
}, "*");
</script></body></html>"#,
    );
    Response::builder()
        .header(CONTENT_TYPE, "text/html")
        .body(Body::from(body))
        .expect("child Shift_JIS document fixture should build")
}

pub(super) async fn encoding_large_static_nodelist_subset_page() -> Html<String> {
    let mut body = String::from("<!doctype html><html><body>");
    for index in 0..12_000 {
        body.push_str("<span data-cp=\"");
        body.push_str(&format!("{index:X}"));
        body.push_str("\" data-bytes=\"A1 A1\">x</span>");
    }
    // Keep the DOM large so eager wrapper creation remains expensive, while the
    // script reads only a small prefix. Traversing every entry would test full
    // iteration instead and make the watchdog outcome depend on host load.
    body.push_str(
        r#"<script>
	try {
	  const started = Date.now();
	  const nodes = document.querySelectorAll("span");
	  let checksum = 0;
	  function simpleDecoder(bytes) { return bytes; }
	  const checksStarted = Date.now();
	  for (let i = 0; i < 10; i++) {
	    checksum += nodes[i].textContent.length;
	    checksum += simpleDecoder(nodes[i].dataset.bytes).length;
	  }
	  const checksElapsed = Date.now() - checksStarted;
	  document.body.setAttribute("data-node-count", String(nodes.length));
	  document.body.setAttribute("data-checksum", String(checksum));
	  document.body.setAttribute("data-checks-elapsed-ms", String(checksElapsed));
	  document.body.setAttribute("data-done", "true");
	  document.body.setAttribute("data-elapsed-ms", String(Date.now() - started));
	} catch (error) {
	  document.body.setAttribute("data-error", String(error && error.stack || error));
	}
	</script></body></html>"#,
    );
    Html(body)
}

pub(super) async fn encoding_large_child_static_nodelist_subset_parent_page() -> Html<&'static str>
{
    Html(
        r#"<!doctype html><html><body><iframe src="/encoding/large-child-static-nodelist-subset-child"></iframe><script>
const started = Date.now();
const frame = document.querySelector("iframe");
frame.addEventListener("load", () => {
  const nodes = frame.contentWindow.document.querySelectorAll("span");
  let checksum = 0;
  for (let i = 0; i < 10; i++) {
    checksum += nodes[i].textContent.length;
    checksum += nodes[i].dataset.bytes.length;
  }
  document.body.setAttribute("data-node-count", String(nodes.length));
  document.body.setAttribute("data-checksum", String(checksum));
  document.body.setAttribute("data-done", "true");
  document.body.setAttribute("data-elapsed-ms", String(Date.now() - started));
});
</script></body></html>"#,
    )
}

pub(super) async fn encoding_large_child_static_nodelist_subset_child_page() -> Html<String> {
    let mut body = String::from("<!doctype html><html><body>");
    for index in 0..12_000 {
        body.push_str("<span data-cp=\"");
        body.push_str(&format!("{index:X}"));
        body.push_str("\" data-bytes=\"A1 A1\">x</span>");
    }
    body.push_str("</body></html>");
    Html(body)
}

pub(super) async fn script_page() -> Html<&'static str> {
    Html(SCRIPT_HTML)
}

pub(super) async fn inline_script_page() -> Html<&'static str> {
    Html(INLINE_SCRIPT_HTML)
}

pub(super) async fn script_execution_page() -> Html<&'static str> {
    Html(SCRIPT_EXECUTION_HTML)
}

pub(super) async fn url_binding_page() -> Html<&'static str> {
    Html(URL_BINDING_HTML)
}

pub(super) async fn selector_corner_cases_page() -> Html<&'static str> {
    Html(SELECTOR_CORNER_CASES_HTML)
}

pub(super) async fn selector_host_bridge_page() -> Html<&'static str> {
    Html(SELECTOR_HOST_BRIDGE_HTML)
}

pub(super) async fn native_bridge_page() -> Html<&'static str> {
    Html(NATIVE_BRIDGE_HTML)
}

pub(super) async fn event_collections_bridge_page() -> Html<&'static str> {
    Html(EVENT_COLLECTIONS_BRIDGE_HTML)
}

pub(super) async fn lifecycle_bridge_page() -> Html<&'static str> {
    Html(LIFECYCLE_BRIDGE_HTML)
}

pub(super) async fn main_document_lifecycle_performance_event_end_page() -> Html<&'static str> {
    Html(MAIN_DOCUMENT_LIFECYCLE_PERFORMANCE_EVENT_END_HTML)
}

pub(super) async fn live_collections_page() -> Html<&'static str> {
    Html(LIVE_COLLECTIONS_HTML)
}

pub(super) async fn tree_bridge_page() -> Html<&'static str> {
    Html(TREE_BRIDGE_HTML)
}

pub(super) async fn rust_dom_source_of_truth_page() -> Html<&'static str> {
    Html(RUST_DOM_SOURCE_OF_TRUTH_HTML)
}

pub(super) async fn rust_dom_lazy_hydration_page() -> Html<&'static str> {
    Html(RUST_DOM_LAZY_HYDRATION_HTML)
}

pub(super) async fn rust_dom_mutation_sync_page() -> Html<&'static str> {
    Html(RUST_DOM_MUTATION_SYNC_HTML)
}

pub(super) async fn rust_dom_fragment_script_sync_page() -> Html<&'static str> {
    Html(RUST_DOM_FRAGMENT_SCRIPT_SYNC_HTML)
}

pub(super) async fn rust_dom_document_open_sync_page() -> Html<&'static str> {
    Html(RUST_DOM_DOCUMENT_OPEN_SYNC_HTML)
}

pub(super) async fn rust_dom_document_open_multiwrite_sync_page() -> Html<&'static str> {
    Html(RUST_DOM_DOCUMENT_OPEN_MULTIWRITE_SYNC_HTML)
}

pub(super) async fn slow_a_page() -> Html<&'static str> {
    sleep(Duration::from_millis(350)).await;
    Html(SLOW_A_HTML)
}

pub(super) async fn slow_b_page() -> Html<&'static str> {
    sleep(Duration::from_millis(350)).await;
    Html(SLOW_B_HTML)
}

pub(super) async fn concurrent_shared_state_a_page() -> Html<String> {
    concurrent_shared_state_page("a").await
}

pub(super) async fn concurrent_shared_state_b_page() -> Html<String> {
    concurrent_shared_state_page("b").await
}

async fn concurrent_shared_state_page(label: &'static str) -> Html<String> {
    let (_request_guard, overlapped) = ConcurrentSharedStateRequestGuard::enter();
    sleep(Duration::from_millis(350)).await;
    Html(format!(
        "<!doctype html><html><body><main>concurrent={label}</main><p>overlap={overlapped}</p></body></html>"
    ))
}

pub(super) async fn streaming_chunked_html_page() -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    tokio::spawn(async move {
        let chunks = vec![
            b"<!doctype html><html><body><main id=\"stream\">naive-\xE4\xBD".to_vec(),
            b"\xA0\xE5".to_vec(),
            b"\xA5\xBD</main><script>document.body.setAttribute('data-stream-script','seen');</script></body></html>".to_vec(),
        ];
        for chunk in chunks {
            if tx
                .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from(chunk)))
                .await
                .is_err()
            {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
    });

    Response::builder()
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from_stream(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .expect("streaming html response should build")
}

pub(super) async fn streaming_slow_html_tail_page() -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel(2);
    tokio::spawn(async move {
        if tx
            .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from_static(
                b"<!doctype html><html><head><title>slow tail</title></head><body><main id=\"early\">early",
            )))
            .await
            .is_err()
        {
            return;
        }
        // Keep initial page creation parked in its streaming-parser
        // continuation long enough for a caller-owned readiness deadline to
        // cancel the fetch and synchronously tear down the browser context.
        sleep(Duration::from_millis(1_000)).await;
        let _ = tx
            .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from_static(
                b"</main></body></html>",
            )))
            .await;
    });

    Response::builder()
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from_stream(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .expect("slow-tail streaming html response should build")
}

pub(super) async fn location_nav_replace_source_page() -> Html<&'static str> {
    Html(LOCATION_NAV_REPLACE_SOURCE_HTML)
}

pub(super) async fn location_nav_assign_source_page() -> Html<&'static str> {
    Html(LOCATION_NAV_ASSIGN_SOURCE_HTML)
}

pub(super) async fn location_nav_href_source_page() -> Html<&'static str> {
    Html(LOCATION_NAV_HREF_SOURCE_HTML)
}

pub(super) async fn location_nav_pathname_source_page() -> Html<&'static str> {
    Html(LOCATION_NAV_PATHNAME_SOURCE_HTML)
}

pub(super) async fn location_nav_pathname_target_page() -> Html<&'static str> {
    Html(
        "<!doctype html><html><body><main id=\"target\">location-target=pathname</main></body></html>",
    )
}

pub(super) async fn location_nav_search_source_page(request: AxumRequest) -> Html<String> {
    let from = request.uri().query().and_then(|query| {
        query
            .split('&')
            .find_map(|entry| entry.strip_prefix("from="))
    });
    match from {
        Some("search") => Html(
            "<!doctype html><html><body><main id=\"target\">location-target=search</main></body></html>"
                .to_owned(),
        ),
        _ => Html(LOCATION_NAV_SEARCH_SOURCE_HTML.to_owned()),
    }
}

pub(super) async fn location_nav_search_async_source_page(request: AxumRequest) -> Html<String> {
    let from = request.uri().query().and_then(|query| {
        query
            .split('&')
            .find_map(|entry| entry.strip_prefix("from="))
    });
    match from {
        Some("search-async") => Html(
            "<!doctype html><html><body><main id=\"target\">location-target=search-async</main></body></html>"
                .to_owned(),
        ),
        _ => Html(LOCATION_NAV_SEARCH_ASYNC_SOURCE_HTML.to_owned()),
    }
}

pub(super) async fn location_nav_host_source_page(request: AxumRequest) -> Html<String> {
    location_nav_host_component_source_page(request, LocationHostComponent::Host)
}

pub(super) async fn location_nav_hostname_source_page(request: AxumRequest) -> Html<String> {
    location_nav_host_component_source_page(request, LocationHostComponent::Hostname)
}

pub(super) async fn location_nav_port_source_page(request: AxumRequest) -> Html<String> {
    let target_port = request.uri().query().and_then(|query| {
        query
            .split('&')
            .find_map(|entry| entry.strip_prefix("targetPort="))
    });
    let host = request
        .headers()
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("127.0.0.1");
    let current_port = host.rsplit_once(':').map(|(_, port)| port);

    if target_port.is_some() && current_port == target_port {
        return Html(
            "<!doctype html><html><body><main id=\"target\">location-target=port</main><script>document.body.setAttribute('data-final-port', location.port);</script></body></html>"
                .to_owned(),
        );
    }

    let setter_value = serde_json::to_string(&target_port.unwrap_or(""))
        .expect("fixture location port setter value should serialize");
    Html(format!(
        "<!doctype html><html><body><main id=\"source\">port-source</main><script>location.port = {setter_value}; window.locationPortAfterCall = location.href;</script></body></html>"
    ))
}

pub(super) async fn location_nav_target_page(request: AxumRequest) -> Html<String> {
    let from = request
        .uri()
        .query()
        .and_then(|query| {
            query
                .split('&')
                .find_map(|entry| entry.strip_prefix("from="))
        })
        .unwrap_or("unknown");
    Html(format!(
        "<!doctype html><html><body><main id=\"target\">location-target={from}</main></body></html>"
    ))
}

pub(super) async fn location_nav_reload_source_page(headers: HeaderMap) -> Response {
    if has_cookie(&headers, "lm-location-reload=1") {
        return Html(
            "<!doctype html><html><body><main id=\"reloaded\">location-reload=done</main></body></html>",
        )
        .into_response();
    }

    (
        [(
            SET_COOKIE,
            "lm-location-reload=1; Path=/location-nav; HttpOnly",
        )],
        Html(LOCATION_NAV_RELOAD_SOURCE_HTML),
    )
        .into_response()
}

pub(super) async fn location_nav_post_load_cookie_reload_challenge_page(
    headers: HeaderMap,
) -> Response {
    if has_cookie(&headers, "lm-post-load-cookie-reload=1") {
        return Html(LOCATION_NAV_POST_LOAD_COOKIE_RELOAD_FINAL_HTML).into_response();
    }

    Html(LOCATION_NAV_POST_LOAD_COOKIE_RELOAD_CHALLENGE_HTML).into_response()
}

pub(super) async fn location_nav_post_load_cookie_reload_final_script() -> Response {
    sleep(Duration::from_millis(120)).await;
    javascript_response("document.documentElement.dataset.finalScript = 'done';")
}

pub(super) async fn location_nav_same_href_cookie_challenge_page(headers: HeaderMap) -> Response {
    if has_cookie(&headers, "EO-Bot-Js-Token=test-token") {
        return Html(LOCATION_NAV_SAME_HREF_COOKIE_CHALLENGE_FINAL_HTML).into_response();
    }

    Html(LOCATION_NAV_SAME_HREF_COOKIE_CHALLENGE_HTML).into_response()
}

pub(super) async fn location_nav_same_href_cookie_challenge_sdk() -> Response {
    Response::builder()
        .header(CONTENT_TYPE, "application/javascript")
        .body(Body::from(LOCATION_NAV_SAME_HREF_COOKIE_CHALLENGE_SDK))
        .expect("same-href cookie challenge SDK fixture should build")
}

pub(super) async fn location_nav_chain_source_page() -> Html<&'static str> {
    Html(LOCATION_NAV_CHAIN_SOURCE_HTML)
}

pub(super) async fn location_nav_chain_mid_page() -> Html<&'static str> {
    Html(LOCATION_NAV_CHAIN_MID_HTML)
}

pub(super) async fn location_nav_chain_timeout_source_page() -> Html<&'static str> {
    Html(LOCATION_NAV_CHAIN_TIMEOUT_SOURCE_HTML)
}

pub(super) async fn location_nav_loop_timeout_source_page() -> Html<&'static str> {
    Html(LOCATION_NAV_LOOP_TIMEOUT_SOURCE_HTML)
}

pub(super) async fn location_nav_loop_a_page() -> Html<&'static str> {
    Html(LOCATION_NAV_LOOP_A_HTML)
}

pub(super) async fn location_nav_loop_b_page() -> Html<&'static str> {
    Html(LOCATION_NAV_LOOP_B_HTML)
}

pub(super) async fn date_locale_bomb_page() -> Html<&'static str> {
    Html(DATE_LOCALE_BOMB_HTML)
}

pub(super) async fn browser_surface_compat_page() -> Html<&'static str> {
    Html(BROWSER_SURFACE_COMPAT_HTML)
}

pub(super) async fn date_locale_details_page() -> Html<&'static str> {
    Html(DATE_LOCALE_DETAILS_HTML)
}

pub(super) async fn browser_surface_details_page() -> Html<&'static str> {
    Html(BROWSER_SURFACE_DETAILS_HTML)
}

pub(super) async fn history_relative_url_update_page() -> Html<&'static str> {
    Html(HISTORY_RELATIVE_URL_UPDATE_HTML)
}

pub(super) async fn history_state_clone_and_dataclone_error_page() -> Html<&'static str> {
    Html(HISTORY_STATE_CLONE_AND_DATACLONE_ERROR_HTML)
}

pub(super) async fn history_cross_origin_security_error_page() -> Html<&'static str> {
    Html(HISTORY_CROSS_ORIGIN_SECURITY_ERROR_HTML)
}

pub(super) async fn history_go_zero_reloads_current_document_page() -> Html<&'static str> {
    Html(HISTORY_GO_ZERO_RELOADS_CURRENT_DOCUMENT_HTML)
}

pub(super) async fn history_go_nan_reloads_current_document_page() -> Html<&'static str> {
    Html(HISTORY_GO_NAN_RELOADS_CURRENT_DOCUMENT_HTML)
}

pub(super) async fn history_go_no_argument_reloads_current_document_page() -> Html<&'static str> {
    Html(HISTORY_GO_NO_ARGUMENT_RELOADS_CURRENT_DOCUMENT_HTML)
}

pub(super) async fn history_go_rejects_symbol_and_bigint_page() -> Html<&'static str> {
    Html(HISTORY_GO_REJECTS_SYMBOL_AND_BIGINT_HTML)
}

pub(super) async fn history_go_string_minus_one_traverses_back_page() -> Html<&'static str> {
    Html(HISTORY_GO_STRING_MINUS_ONE_TRAVERSES_BACK_HTML)
}

pub(super) async fn history_back_same_turn_traverses_asynchronously_page() -> Html<&'static str> {
    Html(HISTORY_BACK_SAME_TURN_TRAVERSES_ASYNCHRONOUSLY_HTML)
}

pub(super) async fn history_back_ignores_page_tampered_queue_microtask_page() -> Html<&'static str>
{
    Html(HISTORY_BACK_IGNORES_PAGE_TAMPERED_QUEUE_MICROTASK_HTML)
}

pub(super) async fn history_back_forward_same_turn_coalesces_page() -> Html<&'static str> {
    Html(HISTORY_BACK_FORWARD_SAME_TURN_COALESCES_HTML)
}

pub(super) async fn history_state_mutation_does_not_mutate_stored_snapshot_page()
-> Html<&'static str> {
    Html(HISTORY_STATE_MUTATION_DOES_NOT_MUTATE_STORED_SNAPSHOT_HTML)
}

pub(super) async fn history_length_and_state_assignments_do_not_mutate_public_surface_page()
-> Html<&'static str> {
    Html(HISTORY_LENGTH_AND_STATE_ASSIGNMENTS_DO_NOT_MUTATE_PUBLIC_SURFACE_HTML)
}

pub(super) async fn history_navigation_brand_and_descriptor_surface_page() -> Html<&'static str> {
    Html(HISTORY_NAVIGATION_BRAND_AND_DESCRIPTOR_SURFACE_HTML)
}

pub(super) async fn history_scroll_restoration_invalid_value_ignored_page() -> Html<&'static str> {
    Html(HISTORY_SCROLL_RESTORATION_INVALID_VALUE_IGNORED_HTML)
}

pub(super) async fn history_pushstate_does_not_set_navigation_current_entry_state_page()
-> Html<&'static str> {
    Html(HISTORY_PUSHSTATE_DOES_NOT_SET_NAVIGATION_CURRENT_ENTRY_STATE_HTML)
}

pub(super) async fn history_location_hash_assignment_dispatches_popstate_and_hashchange_page()
-> Html<&'static str> {
    Html(HISTORY_LOCATION_HASH_ASSIGNMENT_DISPATCHES_POPSTATE_AND_HASHCHANGE_HTML)
}

pub(super) async fn location_nav_assign_post_parse_timeout_source_page() -> Html<&'static str> {
    Html(LOCATION_NAV_ASSIGN_POST_PARSE_TIMEOUT_SOURCE_HTML)
}

pub(super) async fn navigation_currententrychange_on_hash_navigation_page() -> Html<&'static str> {
    Html(NAVIGATION_CURRENTENTRYCHANGE_ON_HASH_NAVIGATION_HTML)
}

pub(super) async fn navigation_currententrychange_ignores_page_tampered_dispatch_event_page()
-> Html<&'static str> {
    Html(NAVIGATION_CURRENTENTRYCHANGE_IGNORES_PAGE_TAMPERED_DISPATCH_EVENT_HTML)
}

pub(super) async fn navigation_update_current_entry_updates_state_and_fires_currententrychange_page()
-> Html<&'static str> {
    Html(NAVIGATION_UPDATE_CURRENT_ENTRY_UPDATES_STATE_AND_FIRES_CURRENTENTRYCHANGE_HTML)
}

pub(super) async fn history_pushstate_dispatches_navigation_currententrychange_event_surface_page()
-> Html<&'static str> {
    Html(HISTORY_PUSHSTATE_DISPATCHES_NAVIGATION_CURRENTENTRYCHANGE_EVENT_SURFACE_HTML)
}

pub(super) async fn navigation_reload_reloads_current_document_page() -> Html<&'static str> {
    Html(NAVIGATION_RELOAD_RELOADS_CURRENT_DOCUMENT_HTML)
}

pub(super) async fn navigation_navigate_same_document_push_updates_history_and_events_page()
-> Html<&'static str> {
    Html(NAVIGATION_NAVIGATE_SAME_DOCUMENT_PUSH_UPDATES_HISTORY_AND_EVENTS_HTML)
}

pub(super) async fn navigation_navigate_same_document_replace_updates_history_and_events_page()
-> Html<&'static str> {
    Html(NAVIGATION_NAVIGATE_SAME_DOCUMENT_REPLACE_UPDATES_HISTORY_AND_EVENTS_HTML)
}

pub(super) async fn navigation_navigate_argument_validation_page() -> Html<&'static str> {
    Html(NAVIGATION_NAVIGATE_ARGUMENT_VALIDATION_HTML)
}

pub(super) async fn navigation_navigate_same_document_result_promises_settle_before_hashchange_page()
-> Html<&'static str> {
    Html(NAVIGATION_NAVIGATE_SAME_DOCUMENT_RESULT_PROMISES_SETTLE_BEFORE_HASHCHANGE_HTML)
}

pub(super) async fn navigation_navigate_same_document_state_uses_structured_clone_page()
-> Html<&'static str> {
    Html(NAVIGATION_NAVIGATE_SAME_DOCUMENT_STATE_USES_STRUCTURED_CLONE_HTML)
}

pub(super) async fn navigation_back_surface_and_fragment_traversal_page() -> Html<&'static str> {
    Html(NAVIGATION_BACK_SURFACE_AND_FRAGMENT_TRAVERSAL_HTML)
}

pub(super) async fn navigation_traverse_to_key_fragment_traversal_page() -> Html<&'static str> {
    Html(NAVIGATION_TRAVERSE_TO_KEY_FRAGMENT_TRAVERSAL_HTML)
}

pub(super) async fn navigation_oncurrententrychange_property_receives_traverse_event_surface_page()
-> Html<&'static str> {
    Html(NAVIGATION_ONCURRENTENTRYCHANGE_PROPERTY_RECEIVES_TRAVERSE_EVENT_SURFACE_HTML)
}

pub(super) async fn navigation_forward_dispatches_currententrychange_traverse_event_surface_page()
-> Html<&'static str> {
    Html(NAVIGATION_FORWARD_DISPATCHES_CURRENTENTRYCHANGE_TRAVERSE_EVENT_SURFACE_HTML)
}

pub(super) async fn navigation_forward_result_promises_settle_after_async_traversal_page()
-> Html<&'static str> {
    Html(NAVIGATION_FORWARD_RESULT_PROMISES_SETTLE_AFTER_ASYNC_TRAVERSAL_HTML)
}

pub(super) async fn navigation_back_result_promises_settle_after_async_traversal_page()
-> Html<&'static str> {
    Html(NAVIGATION_BACK_RESULT_PROMISES_SETTLE_AFTER_ASYNC_TRAVERSAL_HTML)
}

pub(super) async fn navigation_traverse_to_result_promises_settle_after_async_traversal_page()
-> Html<&'static str> {
    Html(NAVIGATION_TRAVERSE_TO_RESULT_PROMISES_SETTLE_AFTER_ASYNC_TRAVERSAL_HTML)
}

pub(super) async fn navigation_back_restores_navigation_entry_state_separately_from_history_state_page()
-> Html<&'static str> {
    Html(NAVIGATION_BACK_RESTORES_NAVIGATION_ENTRY_STATE_SEPARATELY_FROM_HISTORY_STATE_HTML)
}

pub(super) async fn navigation_traverse_to_restores_navigation_entry_state_separately_from_history_state_page()
-> Html<&'static str> {
    Html(NAVIGATION_TRAVERSE_TO_RESTORES_NAVIGATION_ENTRY_STATE_SEPARATELY_FROM_HISTORY_STATE_HTML)
}

pub(super) async fn navigation_navigate_state_persists_to_destination_page() -> Html<&'static str> {
    Html(NAVIGATION_NAVIGATE_STATE_PERSISTS_TO_DESTINATION_HTML)
}

pub(super) async fn navigation_navigate_state_destination_page() -> Html<&'static str> {
    Html(NAVIGATION_NAVIGATE_STATE_DESTINATION_HTML)
}

pub(super) async fn navigation_navigate_cross_document_result_promises_do_not_settle_before_destination_load_page()
-> Html<&'static str> {
    Html(
        NAVIGATION_NAVIGATE_CROSS_DOCUMENT_RESULT_PROMISES_DO_NOT_SETTLE_BEFORE_DESTINATION_LOAD_HTML,
    )
}

pub(super) async fn navigation_navigate_result_promises_destination_page() -> Html<&'static str> {
    Html(NAVIGATION_NAVIGATE_RESULT_PROMISES_DESTINATION_HTML)
}

pub(super) async fn navigation_navigate_cross_document_does_not_dispatch_currententrychange_in_source_page()
-> Html<&'static str> {
    Html(NAVIGATION_NAVIGATE_CROSS_DOCUMENT_DOES_NOT_DISPATCH_CURRENTENTRYCHANGE_IN_SOURCE_HTML)
}

pub(super) async fn navigation_navigate_cross_document_does_not_dispatch_currententrychange_destination_page()
-> Html<&'static str> {
    Html(NAVIGATION_NAVIGATE_CROSS_DOCUMENT_DOES_NOT_DISPATCH_CURRENTENTRYCHANGE_DESTINATION_HTML)
}

pub(super) async fn navigation_activation_initial_surface_page() -> Html<&'static str> {
    Html(NAVIGATION_ACTIVATION_INITIAL_SURFACE_HTML)
}

pub(super) async fn navigation_activation_same_document_navigation_stays_initial_page()
-> Html<&'static str> {
    Html(NAVIGATION_ACTIVATION_SAME_DOCUMENT_NAVIGATION_STAYS_INITIAL_HTML)
}

pub(super) async fn navigation_activation_cross_document_destination_surface_source_page()
-> Html<&'static str> {
    Html(NAVIGATION_ACTIVATION_CROSS_DOCUMENT_DESTINATION_SURFACE_SOURCE_HTML)
}

pub(super) async fn navigation_activation_cross_document_destination_surface_dest_page()
-> Html<&'static str> {
    Html(NAVIGATION_ACTIVATION_CROSS_DOCUMENT_DESTINATION_SURFACE_DEST_HTML)
}

pub(super) async fn navigation_activation_cross_document_back_destination_surface_source_page()
-> Html<&'static str> {
    Html(NAVIGATION_ACTIVATION_CROSS_DOCUMENT_BACK_DESTINATION_SURFACE_SOURCE_HTML)
}

pub(super) async fn navigation_activation_cross_document_back_destination_surface_dest_page()
-> Html<&'static str> {
    Html(NAVIGATION_ACTIVATION_CROSS_DOCUMENT_BACK_DESTINATION_SURFACE_DEST_HTML)
}

pub(super) async fn navigation_activation_cross_document_traverse_to_destination_surface_source_page()
-> Html<&'static str> {
    Html(NAVIGATION_ACTIVATION_CROSS_DOCUMENT_TRAVERSE_TO_DESTINATION_SURFACE_SOURCE_HTML)
}

pub(super) async fn navigation_activation_cross_document_traverse_to_destination_surface_dest_page()
-> Html<&'static str> {
    Html(NAVIGATION_ACTIVATION_CROSS_DOCUMENT_TRAVERSE_TO_DESTINATION_SURFACE_DEST_HTML)
}

pub(super) async fn navigation_navigate_cross_document_push_destination_surface_source_page()
-> Html<&'static str> {
    Html(NAVIGATION_NAVIGATE_CROSS_DOCUMENT_PUSH_DESTINATION_SURFACE_SOURCE_HTML)
}

pub(super) async fn navigation_navigate_cross_document_push_destination_surface_dest_page()
-> Html<&'static str> {
    Html(NAVIGATION_NAVIGATE_CROSS_DOCUMENT_PUSH_DESTINATION_SURFACE_DEST_HTML)
}

pub(super) async fn navigation_entries_expose_current_entry_metadata_and_identity_page()
-> Html<&'static str> {
    Html(NAVIGATION_ENTRIES_EXPOSE_CURRENT_ENTRY_METADATA_AND_IDENTITY_HTML)
}

pub(super) async fn history_initial_navigation_current_entry_index_starts_at_zero_page()
-> Html<&'static str> {
    Html(HISTORY_INITIAL_NAVIGATION_CURRENT_ENTRY_INDEX_STARTS_AT_ZERO_HTML)
}

pub(super) async fn history_onpopstate_property_receives_restored_state_after_back_page()
-> Html<&'static str> {
    Html(HISTORY_ONPOPSTATE_PROPERTY_RECEIVES_RESTORED_STATE_AFTER_BACK_HTML)
}

pub(super) async fn history_back_fragment_traversal_dispatches_popstate_then_hashchange_page()
-> Html<&'static str> {
    Html(HISTORY_BACK_FRAGMENT_TRAVERSAL_DISPATCHES_POPSTATE_THEN_HASHCHANGE_HTML)
}

pub(super) async fn history_forward_fragment_traversal_dispatches_popstate_then_hashchange_page()
-> Html<&'static str> {
    Html(HISTORY_FORWARD_FRAGMENT_TRAVERSAL_DISPATCHES_POPSTATE_THEN_HASHCHANGE_HTML)
}

pub(super) async fn history_location_replace_fragment_replaces_current_entry_page()
-> Html<&'static str> {
    Html(HISTORY_LOCATION_REPLACE_FRAGMENT_REPLACES_CURRENT_ENTRY_HTML)
}

pub(super) async fn canvas_to_data_url_exists_and_handles_zero_size_page() -> Html<&'static str> {
    Html(CANVAS_TO_DATA_URL_EXISTS_AND_HANDLES_ZERO_SIZE_HTML)
}

pub(super) async fn event_handler_accessors_page() -> Html<&'static str> {
    Html(EVENT_HANDLER_ACCESSORS_HTML)
}

pub(super) async fn html_content_accessors_page() -> Html<&'static str> {
    Html(HTML_CONTENT_ACCESSORS_HTML)
}

pub(super) async fn details_dialog_accessors_page() -> Html<&'static str> {
    Html(DETAILS_DIALOG_ACCESSORS_HTML)
}

pub(super) async fn html_element_reflected_accessors_page() -> Html<&'static str> {
    Html(HTML_ELEMENT_REFLECTED_ACCESSORS_HTML)
}

pub(super) async fn style_link_stylesheet_accessors_page() -> Html<&'static str> {
    Html(STYLE_LINK_STYLESHEET_ACCESSORS_HTML)
}

pub(super) async fn script_state_snapshot_handles_throwing_to_primitive_page() -> Html<&'static str>
{
    Html(SCRIPT_STATE_SNAPSHOT_HANDLES_THROWING_TO_PRIMITIVE_HTML)
}

pub(super) async fn script_state_snapshot_ignores_set_prototype_tamper_page() -> Html<&'static str>
{
    Html(SCRIPT_STATE_SNAPSHOT_IGNORES_SET_PROTOTYPE_TAMPER_HTML)
}

pub(super) async fn shadow_dom_slot_template_accessors_page() -> Html<&'static str> {
    Html(SHADOW_DOM_SLOT_TEMPLATE_ACCESSORS_HTML)
}

pub(super) async fn document_has_focus_top_level_true_child_false_page() -> Html<&'static str> {
    Html(DOCUMENT_HAS_FOCUS_TOP_LEVEL_TRUE_CHILD_FALSE_HTML)
}

pub(super) async fn history_initial_child_entry_seed_parent_page() -> Html<&'static str> {
    Html(HISTORY_INITIAL_CHILD_ENTRY_SEED_PARENT_HTML)
}

pub(super) async fn history_initial_child_entry_seed_child_page() -> Html<&'static str> {
    Html(HISTORY_INITIAL_CHILD_ENTRY_SEED_CHILD_HTML)
}

pub(super) async fn window_child_browsing_context_fragment_traversal_events_are_window_local_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_FRAGMENT_TRAVERSAL_EVENTS_ARE_WINDOW_LOCAL_HTML)
}

pub(super) async fn window_child_browsing_context_location_hash_assignment_dispatches_local_popstate_and_hashchange_page()
-> Html<&'static str> {
    Html(
        WINDOW_CHILD_BROWSING_CONTEXT_LOCATION_HASH_ASSIGNMENT_DISPATCHES_LOCAL_POPSTATE_AND_HASHCHANGE_HTML,
    )
}

pub(super) async fn window_child_browsing_context_target_name_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_TARGET_NAME_HTML)
}

pub(super) async fn window_child_browsing_context_window_name_navigation_page() -> Html<&'static str>
{
    Html(
        r#"<!doctype html><html><body>
<pre id="step-log"></pre>
<iframe id="test"></iframe>
<script>
const frame = document.getElementById("test");
const log = document.getElementById("step-log");
const observed = [];
let step = 0;
const steps = [
  () => { frame.src = "/compat/window-child-browsing-context-window-name-a"; },
  () => { observed.push(frame.contentWindow.name); setTimeout(next, 0); },
  () => { frame.src = "/compat/window-child-browsing-context-window-name-b"; },
  () => {
    observed.push(frame.contentWindow.name);
    document.body.dataset.windowNames = JSON.stringify(observed);
  },
];
function next() {
  log.textContent += "\nStep " + step + " " + frame.contentWindow.location;
  steps[step++]();
}
frame.onload = next;
window.onload = () => setTimeout(next, 0);
</script>
</body></html>"#,
    )
}

pub(super) async fn window_child_browsing_context_parse_time_target_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_PARSE_TIME_TARGET_HTML)
}

pub(super) async fn window_child_browsing_context_form_targets_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_FORM_TARGETS_HTML)
}

pub(super) async fn window_child_browsing_context_target_name_a_page() -> Html<&'static str> {
    Html("<!doctype html><html><body>name-a</body></html>")
}

pub(super) async fn window_child_browsing_context_window_name_a_page() -> Html<&'static str> {
    Html(
        "<!doctype html><html><body>window-name-a<script>if (!parent.navigated) { window.name = 'test'; }</script></body></html>",
    )
}

pub(super) async fn window_child_browsing_context_window_name_b_page() -> Html<&'static str> {
    Html(
        "<!doctype html><html><body>window-name-b<script>if (!parent.navigated) { window.name = 'test3'; }</script></body></html>",
    )
}

pub(super) async fn window_child_browsing_context_target_name_id_page() -> Html<&'static str> {
    Html("<!doctype html><html><body>id-hit</body></html>")
}

pub(super) async fn window_child_browsing_context_target_name_old_page() -> Html<&'static str> {
    Html("<!doctype html><html><body>old-name-should-not-hit</body></html>")
}

pub(super) async fn window_child_browsing_context_target_name_b_page() -> Html<&'static str> {
    Html("<!doctype html><html><body>name-b</body></html>")
}

pub(super) async fn window_child_browsing_context_script_globals_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_SCRIPT_GLOBALS_HTML)
}

pub(super) async fn window_child_browsing_context_script_globals_child_page() -> Html<&'static str>
{
    Html(WINDOW_CHILD_BROWSING_CONTEXT_SCRIPT_GLOBALS_CHILD_HTML)
}

pub(super) async fn window_child_browsing_context_history_relative_urls_page() -> Html<&'static str>
{
    Html(WINDOW_CHILD_BROWSING_CONTEXT_HISTORY_RELATIVE_URLS_HTML)
}

pub(super) async fn window_child_browsing_context_fragment_navigation_history_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_FRAGMENT_NAVIGATION_HISTORY_HTML)
}

pub(super) async fn window_child_browsing_context_initial_joint_history_timing_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_INITIAL_JOINT_HISTORY_TIMING_HTML)
}

fn alternate_fixture_host(host: &str) -> String {
    let (hostname, port) = host.rsplit_once(':').unwrap_or((host, ""));
    let alternate_hostname = match hostname {
        "127.0.0.1" => "localhost",
        "localhost" => "127.0.0.1",
        _ => hostname,
    };
    if port.is_empty() {
        alternate_hostname.to_owned()
    } else {
        format!("{alternate_hostname}:{port}")
    }
}

#[derive(Clone, Copy)]
enum LocationHostComponent {
    Host,
    Hostname,
}

impl LocationHostComponent {
    fn name(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Hostname => "hostname",
        }
    }
}

fn location_nav_host_component_source_page(
    request: AxumRequest,
    component: LocationHostComponent,
) -> Html<String> {
    let host = request
        .headers()
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("127.0.0.1");
    let (hostname, _) = host.rsplit_once(':').unwrap_or((host, ""));
    if hostname == "localhost" {
        let name = component.name();
        return Html(format!(
            "<!doctype html><html><body><main id=\"target\">location-target={name}</main><script>document.body.setAttribute('data-final-host', location.host);</script></body></html>"
        ));
    }

    let setter_value = match component {
        LocationHostComponent::Host => alternate_fixture_host(host),
        LocationHostComponent::Hostname => "localhost".to_owned(),
    };
    let setter_value = serde_json::to_string(&setter_value)
        .expect("fixture location host setter value should serialize");
    let name = component.name();
    Html(format!(
        "<!doctype html><html><body><main id=\"source\">{name}-source</main><script>location.{name} = {setter_value}; window.locationHostComponentAfterCall = location.href;</script></body></html>"
    ))
}

pub(super) async fn window_child_browsing_context_external_script_cookie_parent_page(
    request: AxumRequest,
) -> Response {
    let host = request
        .headers()
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost");
    let child_origin = format!("http://{}", alternate_fixture_host(host));
    (
        [(SET_COOKIE, "lm-child-script-lax=1; Path=/; SameSite=Lax")],
        Html(format!(
            "<!doctype html><html><body><iframe id=\"child\" src=\"{child_origin}/compat/window-child-browsing-context-external-script-cookie-child\"></iframe><script>const iframe = document.getElementById('child');window.addEventListener('message', event => {{ if (event.data && event.data.type === 'external-cookie-seen') {{ document.body.setAttribute('data-child-cookie-seen', String(event.data.value)); document.body.setAttribute('data-child-parent-is-top', String(event.data.parentIsTop)); document.body.setAttribute('data-child-length', String(event.data.childLength)); document.body.setAttribute('data-child-parent-length', String(event.data.parentLength)); }} }});iframe.addEventListener('load', () => {{document.body.setAttribute('data-child-ready', 'true');document.body.setAttribute('data-child-content-document', String(iframe.contentDocument));const count = Number(document.body.getAttribute('data-child-load-count') || '0') + 1;document.body.setAttribute('data-child-load-count', String(count));}});</script></body></html>"
        )),
    )
        .into_response()
}

pub(super) async fn window_child_browsing_context_external_script_cookie_child_page(
    request: AxumRequest,
) -> Html<String> {
    let host = request
        .headers()
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("127.0.0.1");
    let script_origin = format!("http://{}", alternate_fixture_host(host));
    Html(format!(
        "<!doctype html><html><body><iframe id=\"nested\" name=\"nestedNamed\" srcdoc=\"<p>nested</p>\"></iframe><iframe id=\"document-collision\" name=\"document\" srcdoc=\"<p>document collision</p>\"></iframe><iframe id=\"focus-collision\" name=\"focus\" srcdoc=\"<p>focus collision</p>\"></iframe><script src=\"{script_origin}/compat/window-child-browsing-context-external-script-cookie.js\"></script><script>addEventListener('message', event => {{ if (event.data && event.data.type === 'report-length') {{ parent.postMessage({{type:event.data.replyType,parentLength:parent.length,topLength:top.length}}, '*'); }} }});document.body.setAttribute('data-external-cookie-seen', String(window.externalChildCookieSeen));parent.postMessage({{type:'external-cookie-seen',value:window.externalChildCookieSeen,parentIsTop:parent === top,childLength:length,parentLength:parent.length}}, '*');</script></body></html>"
    ))
}

pub(super) async fn window_child_browsing_context_external_script_cookie_asset(
    headers: HeaderMap,
) -> Response {
    let source = if has_cookie(&headers, "lm-child-script-lax=1") {
        "window.externalChildCookieSeen = true;"
    } else {
        "window.externalChildCookieSeen = false;"
    };
    javascript_response(source)
}

pub(super) async fn window_crypto_page() -> Html<&'static str> {
    Html(WINDOW_CRYPTO_HTML)
}

pub(super) async fn window_css_page() -> Html<&'static str> {
    Html(WINDOW_CSS_HTML)
}

pub(super) async fn servo_match_media_parsing_page() -> Html<&'static str> {
    Html(SERVO_MATCH_MEDIA_PARSING_HTML)
}

pub(super) async fn servo_style_attr_braces_page() -> Html<&'static str> {
    Html(SERVO_STYLE_ATTR_BRACES_HTML)
}

pub(super) async fn servo_style_attr_urls_page() -> Html<&'static str> {
    Html(SERVO_STYLE_ATTR_URLS_HTML)
}

pub(super) async fn servo_query_is_page() -> Html<&'static str> {
    Html(SERVO_QUERY_IS_HTML)
}

pub(super) async fn servo_query_where_page() -> Html<&'static str> {
    Html(SERVO_QUERY_WHERE_HTML)
}

pub(super) async fn servo_match_media_case_insensitive_page() -> Html<&'static str> {
    Html(SERVO_MATCH_MEDIA_CASE_INSENSITIVE_HTML)
}

pub(super) async fn servo_match_media_invalid_types_page() -> Html<&'static str> {
    Html(SERVO_MATCH_MEDIA_INVALID_TYPES_HTML)
}

pub(super) async fn servo_match_media_feature_states_page() -> Html<&'static str> {
    Html(SERVO_MATCH_MEDIA_FEATURE_STATES_HTML)
}

pub(super) async fn servo_match_media_aspect_ratio_serialization_page() -> Html<&'static str> {
    Html(SERVO_MATCH_MEDIA_ASPECT_RATIO_SERIALIZATION_HTML)
}

pub(super) async fn servo_match_media_preferences_page() -> Html<&'static str> {
    Html(SERVO_MATCH_MEDIA_PREFERENCES_HTML)
}

pub(super) async fn servo_media_query_list_event_target_page() -> Html<&'static str> {
    Html(SERVO_MEDIA_QUERY_LIST_EVENT_TARGET_HTML)
}

pub(super) async fn servo_css_supports_conditions_page() -> Html<&'static str> {
    Html(SERVO_CSS_SUPPORTS_CONDITIONS_HTML)
}

pub(super) async fn servo_fontfaceset_historical_page() -> Html<&'static str> {
    Html(SERVO_FONTFACESET_HISTORICAL_HTML)
}

pub(super) async fn servo_fontfaceset_connected_page() -> Html<&'static str> {
    Html(SERVO_FONTFACESET_CONNECTED_HTML)
}

pub(super) async fn servo_fontfaceset_connected_ignore_page_tampered_style_queries_page()
-> Html<&'static str> {
    Html(SERVO_FONTFACESET_CONNECTED_IGNORE_PAGE_TAMPERED_STYLE_QUERIES_HTML)
}

pub(super) async fn servo_fontfaceset_connected_clear_delete_page() -> Html<&'static str> {
    Html(SERVO_FONTFACESET_CONNECTED_CLEAR_DELETE_HTML)
}

pub(super) async fn servo_fontfaceset_has_page() -> Html<&'static str> {
    Html(SERVO_FONTFACESET_HAS_HTML)
}

pub(super) async fn servo_fontfaceset_delete_clear_css_connected_page() -> Html<&'static str> {
    Html(SERVO_FONTFACESET_DELETE_CLEAR_CSS_CONNECTED_HTML)
}

pub(super) async fn servo_fontfaceset_load_ready_page() -> Html<&'static str> {
    Html(SERVO_FONTFACESET_LOAD_READY_HTML)
}

pub(super) async fn servo_fontfaceset_empty_family_load_page() -> Html<&'static str> {
    Html(SERVO_FONTFACESET_EMPTY_FAMILY_LOAD_HTML)
}

pub(super) async fn servo_fontfaceset_no_root_element_page() -> Html<&'static str> {
    Html(SERVO_FONTFACESET_NO_ROOT_ELEMENT_HTML)
}

pub(super) async fn servo_fontfaceset_update_after_stylesheet_change_page() -> Html<&'static str> {
    Html(SERVO_FONTFACESET_UPDATE_AFTER_STYLESHEET_CHANGE_HTML)
}

pub(super) async fn servo_fontfaceset_load_css_connected_page() -> Html<&'static str> {
    Html(SERVO_FONTFACESET_LOAD_CSS_CONNECTED_HTML)
}

pub(super) async fn chrome_media_query_list_add_remove_listener_page() -> Html<&'static str> {
    Html(CHROME_MEDIA_QUERY_LIST_ADD_REMOVE_LISTENER_HTML)
}

pub(super) async fn chrome_css_escape_dom_api_page() -> Html<&'static str> {
    Html(CHROME_CSS_ESCAPE_DOM_API_HTML)
}

pub(super) async fn chrome_stylesheetlist_style_only_page() -> Html<&'static str> {
    Html(CHROME_STYLESHEETLIST_STYLE_ONLY_HTML)
}

pub(super) async fn chrome_stylesheetlist_mixed_disabled_page() -> Html<&'static str> {
    Html(CHROME_STYLESHEETLIST_MIXED_DISABLED_HTML)
}

pub(super) async fn chrome_stylesheetlist_item_page() -> Html<&'static str> {
    Html(CHROME_STYLESHEETLIST_ITEM_HTML)
}

pub(super) async fn chrome_cssom_missing_arguments_page() -> Html<&'static str> {
    Html(CHROME_CSSOM_MISSING_ARGUMENTS_HTML)
}

pub(super) async fn chrome_cssfloat_cssom_page() -> Html<&'static str> {
    Html(CHROME_CSSFLOAT_CSSOM_HTML)
}

pub(super) async fn chrome_overflow_property_page() -> Html<&'static str> {
    Html(CHROME_OVERFLOW_PROPERTY_HTML)
}

pub(super) async fn chrome_cssstylesheet_rule_mutation_page() -> Html<&'static str> {
    Html(CHROME_CSSSTYLESHEET_RULE_MUTATION_HTML)
}

pub(super) async fn chrome_delete_rule_no_crash_page() -> Html<&'static str> {
    Html(CHROME_DELETE_RULE_NO_CRASH_HTML)
}

pub(super) async fn chrome_important_js_override_page() -> Html<&'static str> {
    Html(CHROME_IMPORTANT_JS_OVERRIDE_HTML)
}

pub(super) async fn chrome_box_sizing_backwards_compat_page() -> Html<&'static str> {
    Html(CHROME_BOX_SIZING_BACKWARDS_COMPAT_HTML)
}

pub(super) async fn chrome_css_supports_dom_api_page() -> Html<&'static str> {
    Html(CHROME_CSS_SUPPORTS_DOM_API_HTML)
}

pub(super) async fn chrome_css_supports_shorthands_page() -> Html<&'static str> {
    Html(CHROME_CSS_SUPPORTS_SHORTHANDS_HTML)
}

pub(super) async fn chrome_css_supports_syntax_page() -> Html<&'static str> {
    Html(CHROME_CSS_SUPPORTS_SYNTAX_HTML)
}

pub(super) async fn chrome_stylesheetlist_1_css() -> Response {
    (
        [(
            CONTENT_TYPE,
            HeaderValue::from_static("text/css; charset=utf-8"),
        )],
        CHROME_STYLESHEETLIST_1_CSS,
    )
        .into_response()
}

pub(super) async fn chrome_stylesheetlist_2_css() -> Response {
    (
        [(
            CONTENT_TYPE,
            HeaderValue::from_static("text/css; charset=utf-8"),
        )],
        CHROME_STYLESHEETLIST_2_CSS,
    )
        .into_response()
}

pub(super) async fn chrome_stylesheetlist_3_css() -> Response {
    (
        [(
            CONTENT_TYPE,
            HeaderValue::from_static("text/css; charset=utf-8"),
        )],
        CHROME_STYLESHEETLIST_3_CSS,
    )
        .into_response()
}

pub(super) async fn chrome_css_supports_coercion_page() -> Html<&'static str> {
    Html(CHROME_CSS_SUPPORTS_COERCION_HTML)
}

pub(super) async fn chrome_fontfaceset_basic_page() -> Html<&'static str> {
    Html(CHROME_FONTFACESET_BASIC_HTML)
}

pub(super) async fn chrome_fontfaceset_iteration_page() -> Html<&'static str> {
    Html(CHROME_FONTFACESET_ITERATION_HTML)
}

pub(super) async fn chrome_fontfaceset_platform_fonts_page() -> Html<&'static str> {
    Html(CHROME_FONTFACESET_PLATFORM_FONTS_HTML)
}

pub(super) async fn chrome_fontfaceset_events_subset_page() -> Html<&'static str> {
    Html(CHROME_FONTFACESET_EVENTS_SUBSET_HTML)
}

pub(super) async fn chrome_fontfaceset_set_operations_subset_page() -> Html<&'static str> {
    Html(CHROME_FONTFACESET_SET_OPERATIONS_SUBSET_HTML)
}

pub(super) async fn chrome_fontfaceset_detached_frame_ready_page() -> Html<&'static str> {
    Html(CHROME_FONTFACESET_DETACHED_FRAME_READY_HTML)
}

pub(super) async fn chrome_fontfaceset_ready_basic_page() -> Html<&'static str> {
    Html(CHROME_FONTFACESET_READY_BASIC_HTML)
}

pub(super) async fn chrome_fontfaceset_invalid_family_names_page() -> Html<&'static str> {
    Html(CHROME_FONTFACESET_INVALID_FAMILY_NAMES_HTML)
}

pub(super) async fn chrome_fontfaceset_unattached_document_page() -> Html<&'static str> {
    Html(CHROME_FONTFACESET_UNATTACHED_DOCUMENT_HTML)
}

pub(super) async fn chrome_webfont_insert_rule_no_crash_page() -> Html<&'static str> {
    Html(CHROME_WEBFONT_INSERT_RULE_NO_CRASH_HTML)
}

pub(super) async fn document_fonts_events_page() -> Html<&'static str> {
    Html(DOCUMENT_FONTS_EVENTS_HTML)
}

pub(super) async fn window_host_globals_page() -> Html<&'static str> {
    Html(WINDOW_HOST_GLOBALS_HTML)
}

pub(super) async fn window_child_browsing_context_length_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_LENGTH_HTML)
}

pub(super) async fn window_child_browsing_context_snapshot_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_SNAPSHOT_HTML)
}

pub(super) async fn window_child_browsing_context_post_message_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_POST_MESSAGE_HTML)
}

pub(super) async fn window_child_browsing_context_window_graph_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_WINDOW_GRAPH_HTML)
}

pub(super) async fn window_child_browsing_context_runtime_backing_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_RUNTIME_BACKING_HTML)
}

pub(super) async fn window_child_browsing_context_location_navigation_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_LOCATION_NAVIGATION_HTML)
}

pub(super) async fn window_child_browsing_context_location_pathname_pending_document_coherence_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_LOCATION_PATHNAME_PENDING_DOCUMENT_COHERENCE_HTML)
}

pub(super) async fn window_child_browsing_context_attribute_navigation_history_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_ATTRIBUTE_NAVIGATION_HISTORY_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_state_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_STATE_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_push_state_page() -> Html<&'static str>
{
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_PUSH_STATE_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_back_cross_document_destination_surface_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_BACK_CROSS_DOCUMENT_DESTINATION_SURFACE_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_noop_result_surface_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_NOOP_RESULT_SURFACE_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_traverse_to_noop_result_surface_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_TRAVERSE_TO_NOOP_RESULT_SURFACE_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_same_document_push_result_surface_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_SAME_DOCUMENT_PUSH_RESULT_SURFACE_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_same_document_push_result_surface_child_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_SAME_DOCUMENT_PUSH_RESULT_SURFACE_CHILD_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_same_document_replace_result_surface_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_SAME_DOCUMENT_REPLACE_RESULT_SURFACE_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_same_document_replace_result_surface_child_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_SAME_DOCUMENT_REPLACE_RESULT_SURFACE_CHILD_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_push_result_surface_in_child_script_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_PUSH_RESULT_SURFACE_IN_CHILD_SCRIPT_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_push_result_surface_source_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_PUSH_RESULT_SURFACE_SOURCE_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_push_result_surface_destination_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_PUSH_RESULT_SURFACE_DESTINATION_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_result_surface_in_child_script_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_RESULT_SURFACE_IN_CHILD_SCRIPT_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_result_surface_source_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_RESULT_SURFACE_SOURCE_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_result_surface_destination_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_RESULT_SURFACE_DESTINATION_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_pending_document_coherence_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_PENDING_DOCUMENT_COHERENCE_HTML)
}

pub(super) async fn window_child_browsing_context_reload_result_surface_in_child_script_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_RELOAD_RESULT_SURFACE_IN_CHILD_SCRIPT_HTML)
}

pub(super) async fn window_child_browsing_context_reload_result_surface_child_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_RELOAD_RESULT_SURFACE_CHILD_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_reload_pending_document_coherence_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_RELOAD_PENDING_DOCUMENT_COHERENCE_HTML)
}

pub(super) async fn window_child_browsing_context_location_reload_pending_document_coherence_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_LOCATION_RELOAD_PENDING_DOCUMENT_COHERENCE_HTML)
}

pub(super) async fn window_child_browsing_context_history_go_zero_pending_document_coherence_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_HISTORY_GO_ZERO_PENDING_DOCUMENT_COHERENCE_HTML)
}

pub(super) async fn window_child_browsing_context_traversal_pending_document_coherence_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_TRAVERSAL_PENDING_DOCUMENT_COHERENCE_HTML)
}

pub(super) async fn window_child_browsing_context_forward_traversal_pending_document_coherence_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_FORWARD_TRAVERSAL_PENDING_DOCUMENT_COHERENCE_HTML)
}

pub(super) async fn window_child_browsing_context_current_entry_same_document_uses_child_owner_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_CURRENT_ENTRY_SAME_DOCUMENT_USES_CHILD_OWNER_HTML)
}

pub(super) async fn window_child_browsing_context_history_popstate_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_HISTORY_POPSTATE_HTML)
}

pub(super) async fn window_child_browsing_context_history_go_one_fragment_traversal_events_are_window_local_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_HISTORY_GO_ONE_FRAGMENT_TRAVERSAL_EVENTS_ARE_WINDOW_LOCAL_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_forward_currententrychange_traverse_event_surface_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_FORWARD_CURRENTENTRYCHANGE_TRAVERSE_EVENT_SURFACE_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_forward_result_promises_are_window_local_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_FORWARD_RESULT_PROMISES_ARE_WINDOW_LOCAL_HTML)
}

pub(super) async fn window_child_browsing_context_currententrychange_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_CURRENTENTRYCHANGE_HTML)
}

pub(super) async fn window_child_browsing_context_activation_same_document_navigation_stays_initial_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_ACTIVATION_SAME_DOCUMENT_NAVIGATION_STAYS_INITIAL_HTML)
}

pub(super) async fn window_child_browsing_context_iframe_load_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_IFRAME_LOAD_HTML)
}

pub(super) async fn window_child_browsing_context_navigation_identity_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_IDENTITY_HTML)
}

pub(super) async fn window_child_browsing_context_redirect_coherence_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_REDIRECT_COHERENCE_HTML)
}

pub(super) async fn window_child_browsing_context_redirect_child_start_page() -> Redirect {
    Redirect::temporary("/compat/window-child-browsing-context-redirect-child-final")
}

pub(super) async fn window_child_browsing_context_redirect_child_final_page() -> Html<&'static str>
{
    Html("<!doctype html><html><body data-child=\"redirect-final\"></body></html>")
}

pub(super) async fn window_child_browsing_context_delayed_async_navigation_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_DELAYED_ASYNC_NAVIGATION_HTML)
}

pub(super) async fn window_child_browsing_context_pending_navigation_coherence_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_PENDING_NAVIGATION_COHERENCE_HTML)
}

pub(super) async fn window_child_browsing_context_delayed_external_script_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_DELAYED_EXTERNAL_SCRIPT_HTML)
}

pub(super) async fn window_child_browsing_context_stale_async_navigation_page() -> Html<&'static str>
{
    Html(WINDOW_CHILD_BROWSING_CONTEXT_STALE_ASYNC_NAVIGATION_HTML)
}

pub(super) async fn window_child_browsing_context_stale_external_script_page() -> Html<&'static str>
{
    Html(WINDOW_CHILD_BROWSING_CONTEXT_STALE_EXTERNAL_SCRIPT_HTML)
}

pub(super) async fn window_child_browsing_context_disconnected_async_navigation_page()
-> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_DISCONNECTED_ASYNC_NAVIGATION_HTML)
}

pub(super) async fn window_child_browsing_context_delayed_child_page() -> Html<&'static str> {
    sleep(Duration::from_millis(120)).await;
    Html(
        "<!doctype html><html><body data-child=\"delayed\"><script>document.body.setAttribute('data-script-ran','true');</script></body></html>",
    )
}

pub(super) async fn window_child_browsing_context_delayed_external_script_child_page()
-> Html<&'static str> {
    Html(
        "<!doctype html><html><body data-child=\"delayed-script\"><script src=\"/compat/window-child-browsing-context-delayed-external-script.js\"></script><script>window.childDelayedScriptOrder.push('inline');document.body.setAttribute('data-inline-after-external', String(window.childDelayedExternalReady === true));document.body.setAttribute('data-script-order', window.childDelayedScriptOrder.join(','));</script></body></html>",
    )
}

pub(super) async fn window_child_browsing_context_delayed_external_script_asset() -> Response {
    sleep(Duration::from_millis(120)).await;
    javascript_response(
        "window.childDelayedScriptOrder = ['external'];window.childDelayedExternalReady = true;document.body.setAttribute('data-external-script','done');",
    )
}

pub(super) async fn window_child_browsing_context_stale_slow_child_page() -> Html<&'static str> {
    sleep(Duration::from_millis(220)).await;
    Html("<!doctype html><html><body data-child=\"slow\"></body></html>")
}

pub(super) async fn window_child_browsing_context_stale_fast_child_page() -> Html<&'static str> {
    sleep(Duration::from_millis(20)).await;
    Html("<!doctype html><html><body data-child=\"fast\"></body></html>")
}

pub(super) async fn window_child_browsing_context_stale_script_slow_child_page()
-> Html<&'static str> {
    Html(
        "<!doctype html><html><body data-child=\"slow\"><script src=\"/compat/window-child-browsing-context-stale-script-slow.js\"></script><script>document.body.setAttribute('data-child','slow-inline');</script></body></html>",
    )
}

pub(super) async fn window_child_browsing_context_stale_script_fast_child_page()
-> Html<&'static str> {
    sleep(Duration::from_millis(20)).await;
    Html("<!doctype html><html><body data-child=\"fast\"></body></html>")
}

pub(super) async fn window_child_browsing_context_stale_script_slow_asset() -> Response {
    sleep(Duration::from_millis(220)).await;
    javascript_response(
        "window.top.document.body.setAttribute('data-stale-script-ran','true');document.body.setAttribute('data-stale-script-ran','true');",
    )
}

pub(super) async fn window_child_browsing_context_disconnected_slow_child_page()
-> Html<&'static str> {
    sleep(Duration::from_millis(120)).await;
    Html("<!doctype html><html><body data-child=\"removed\"></body></html>")
}

pub(super) async fn window_child_browsing_context_post_message_origin_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_POST_MESSAGE_ORIGIN_HTML)
}

pub(super) async fn window_child_browsing_context_post_message_origin_child_page()
-> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script>
      window.addEventListener('message', event => {
        event.source.postMessage(
          { type: 'echo', value: String(event.data) },
          { targetOrigin: event.origin }
        );
      });
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn window_child_browsing_context_post_message_cross_origin_reply_page()
-> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body
    data-response="pending"
    data-source-ok="false"
    data-child-source-ok="false"
    data-last-source-ok=""
    data-last-type=""
    data-child-origin=""
    data-child-loaded="false"
  >
    <script>
      const iframe = document.createElement('iframe');
      const childOrigin = location.origin.replace('127.0.0.1', 'localhost');
      iframe.src = childOrigin + '/compat/window-child-browsing-context-post-message-cross-origin-reply-child';
      document.body.appendChild(iframe);
      const childWindow = iframe.contentWindow;
      window.addEventListener('message', event => {
        document.body.dataset.lastSourceOk = String(event.source === iframe.contentWindow);
        document.body.dataset.lastType = String(event.data?.type ?? '');
        if (
          event.source === iframe.contentWindow &&
          event.data?.type === 'response' &&
          event.data?.requestId === 'r1'
        ) {
          document.body.dataset.response = event.data.token;
          document.body.dataset.sourceOk = String(event.source === iframe.contentWindow);
          document.body.dataset.childSourceOk = String(event.data.sourceIsParent);
          document.body.dataset.childOrigin = String(event.data.origin);
        }
      });
      iframe.addEventListener('load', () => {
        document.body.dataset.childLoaded = 'true';
        childWindow.postMessage({ type: 'token', requestId: 'r1' }, childOrigin);
      });
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn window_child_browsing_context_post_message_cross_origin_reply_child_page()
-> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script>
      window.addEventListener('message', event => {
        try {
          fetch('/compat/window-child-browsing-context-post-message-cross-origin-reply-token')
            .then(response => response.text())
            .then(token => {
              event.source.postMessage(
                {
                  type: 'response',
                  requestId: event.data?.requestId,
                  token: token.trim(),
                  sourceIsParent: event.source === parent,
                  origin: event.origin
                },
                { targetOrigin: event.origin }
              );
            })
            .catch(error => {
              event.source.postMessage(
                { type: 'error', requestId: event.data?.requestId, message: String(error) },
                { targetOrigin: event.origin }
              );
            });
        } catch (error) {
          event.source.postMessage(
            { type: 'throw', requestId: event.data?.requestId, message: String(error) },
            { targetOrigin: event.origin }
          );
        }
      });
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn window_child_browsing_context_post_message_cross_origin_reply_token_page()
-> Html<&'static str> {
    Html("ok")
}

pub(super) async fn window_child_browsing_context_worker_relay_parent_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_WORKER_RELAY_PARENT_HTML)
}

pub(super) async fn window_child_browsing_context_worker_relay_child_page() -> Html<&'static str> {
    Html(WINDOW_CHILD_BROWSING_CONTEXT_WORKER_RELAY_CHILD_HTML)
}

pub(super) async fn window_child_browsing_context_worker_relay_worker_asset() -> Response {
    javascript_response("postMessage({message:'worker-ready',pathname:location.pathname});")
}

pub(super) async fn navigator_extended_page() -> Html<&'static str> {
    Html(NAVIGATOR_EXTENDED_HTML)
}

pub(super) async fn event_bubbles_page() -> Html<&'static str> {
    Html(EVENT_BUBBLES_HTML)
}

pub(super) async fn event_listener_exception_dispatch_page() -> Html<&'static str> {
    Html(EVENT_LISTENER_EXCEPTION_DISPATCH_HTML)
}

pub(super) async fn custom_element_callback_exception_page() -> Html<&'static str> {
    Html(CUSTOM_ELEMENT_CALLBACK_EXCEPTION_HTML)
}

pub(super) async fn local_event_target_callback_exception_page() -> Html<&'static str> {
    Html(LOCAL_EVENT_TARGET_CALLBACK_EXCEPTION_HTML)
}

pub(super) async fn sync_foreach_callback_exception_page() -> Html<&'static str> {
    Html(SYNC_FOREACH_CALLBACK_EXCEPTION_HTML)
}

pub(super) async fn window_named_access_page() -> Html<&'static str> {
    Html(WINDOW_NAMED_ACCESS_HTML)
}

pub(super) async fn window_match_media_page() -> Html<&'static str> {
    Html(WINDOW_MATCH_MEDIA_HTML)
}

pub(super) async fn window_screen_events_page() -> Html<&'static str> {
    Html(WINDOW_SCREEN_EVENTS_HTML)
}

pub(super) async fn uncaught_script_error_page() -> Html<&'static str> {
    Html(UNCAUGHT_SCRIPT_ERROR_HTML)
}

pub(super) async fn load_listener_error_page() -> Html<&'static str> {
    Html(LOAD_LISTENER_ERROR_HTML)
}

pub(super) async fn handled_promise_rejection_page() -> Html<&'static str> {
    Html(HANDLED_PROMISE_REJECTION_HTML)
}

pub(super) async fn unhandled_promise_rejection_page() -> Html<&'static str> {
    Html(UNHANDLED_PROMISE_REJECTION_HTML)
}

pub(super) async fn caught_dynamic_bare_import_page() -> Html<&'static str> {
    Html(CAUGHT_DYNAMIC_BARE_IMPORT_HTML)
}

pub(super) async fn queue_microtask_ignores_promise_tamper_page() -> Html<&'static str> {
    Html(QUEUE_MICROTASK_IGNORES_PROMISE_TAMPER_HTML)
}

pub(super) async fn post_message_ignores_page_tampered_queue_microtask_page() -> Html<&'static str>
{
    Html(POST_MESSAGE_IGNORES_PAGE_TAMPERED_QUEUE_MICROTASK_HTML)
}

pub(super) async fn message_port_ignores_page_tampered_queue_microtask_page() -> Html<&'static str>
{
    Html(MESSAGE_PORT_IGNORES_PAGE_TAMPERED_QUEUE_MICROTASK_HTML)
}

pub(super) async fn mutation_observer_ignores_page_tampered_queue_microtask_page()
-> Html<&'static str> {
    Html(MUTATION_OBSERVER_IGNORES_PAGE_TAMPERED_QUEUE_MICROTASK_HTML)
}

pub(super) async fn message_port_callback_error_page() -> Html<&'static str> {
    Html(MESSAGE_PORT_CALLBACK_ERROR_HTML)
}

pub(super) async fn file_reader_callback_error_page() -> Html<&'static str> {
    Html(FILE_READER_CALLBACK_ERROR_HTML)
}

pub(super) async fn mutation_observer_callback_error_page() -> Html<&'static str> {
    Html(MUTATION_OBSERVER_CALLBACK_ERROR_HTML)
}

pub(super) async fn resize_observer_callback_error_page() -> Html<&'static str> {
    Html(RESIZE_OBSERVER_CALLBACK_ERROR_HTML)
}

pub(super) async fn xhr_ignores_page_tampered_queue_microtask_page() -> Html<&'static str> {
    Html(XHR_IGNORES_PAGE_TAMPERED_QUEUE_MICROTASK_HTML)
}

pub(super) async fn xhr_callback_error_page() -> Html<&'static str> {
    Html(XHR_CALLBACK_ERROR_HTML)
}

pub(super) async fn abort_signal_callback_error_page() -> Html<&'static str> {
    Html(ABORT_SIGNAL_CALLBACK_ERROR_HTML)
}

pub(super) async fn dump_dom_snapshot_page() -> Html<&'static str> {
    Html(DUMP_DOM_SNAPSHOT_HTML)
}

pub(super) async fn parse_time_inline_classic_page() -> Html<&'static str> {
    Html(PARSE_TIME_INLINE_CLASSIC_HTML)
}

pub(super) async fn parse_time_external_classic_page() -> Html<&'static str> {
    Html(PARSE_TIME_EXTERNAL_CLASSIC_HTML)
}

pub(super) async fn script_src_base_alpha_page() -> Html<&'static str> {
    Html(SCRIPT_SRC_BASE_ALPHA_HTML)
}

pub(super) async fn parse_time_defer_classic_page() -> Html<&'static str> {
    Html(PARSE_TIME_DEFER_CLASSIC_HTML)
}

pub(super) async fn parse_time_async_classic_page() -> Html<&'static str> {
    Html(PARSE_TIME_ASYNC_CLASSIC_HTML)
}

pub(super) async fn parse_time_async_classic_chunked_page(headers: HeaderMap) -> Response {
    let split_marker = "<div id=\"late\">late</div>";
    let split_index = PARSE_TIME_ASYNC_CLASSIC_CHUNKED_HTML
        .find(split_marker)
        .expect("chunked async classic fixture must include late DOM marker");
    let (head, tail) = PARSE_TIME_ASYNC_CLASSIC_CHUNKED_HTML.split_at(split_index);
    let head = head.as_bytes().to_vec();
    let tail = tail.as_bytes().to_vec();
    let host_key = request_host_key(&headers).unwrap_or_default();
    let tail_gate = parse_time_async_chunked_tail_gate(&host_key);
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    tokio::spawn(async move {
        if tx
            .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from(head)))
            .await
            .is_err()
        {
            return;
        }
        let _ = tokio::time::timeout(Duration::from_millis(500), tail_gate.notified()).await;
        sleep(Duration::from_millis(20)).await;
        let _ = tx
            .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from(tail)))
            .await;
        if !host_key.is_empty() {
            remove_parse_time_async_chunked_tail_gate(&host_key);
        }
    });

    Response::builder()
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from_stream(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .expect("chunked async classic response should build")
}

pub(super) async fn parse_time_async_classic_slow_chunked_tail_page() -> Response {
    let split_marker = "<script defer";
    let split_index = PARSE_TIME_ASYNC_CLASSIC_SLOW_CHUNKED_TAIL_HTML
        .find(split_marker)
        .expect("slow chunked async fixture must include tail defer script");
    let (head, tail) = PARSE_TIME_ASYNC_CLASSIC_SLOW_CHUNKED_TAIL_HTML.split_at(split_index);
    let head = head.as_bytes().to_vec();
    let tail = tail.as_bytes().to_vec();
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    tokio::spawn(async move {
        if tx
            .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from(head)))
            .await
            .is_err()
        {
            return;
        }
        sleep(Duration::from_millis(5)).await;
        let _ = tx
            .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from(tail)))
            .await;
    });

    Response::builder()
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from_stream(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .expect("slow chunked async classic response should build")
}

pub(super) async fn parse_time_async_classic_pumped_page() -> Html<&'static str> {
    Html(PARSE_TIME_ASYNC_CLASSIC_PUMPED_HTML)
}

pub(super) async fn parse_time_async_classic_slow_page() -> Html<&'static str> {
    Html(PARSE_TIME_ASYNC_CLASSIC_SLOW_HTML)
}

pub(super) async fn parse_time_async_classic_task_turns_page() -> Html<&'static str> {
    Html(PARSE_TIME_ASYNC_CLASSIC_TASK_TURNS_HTML)
}

pub(super) async fn parse_time_async_classic_task_turn_visibility_page() -> Html<&'static str> {
    Html(PARSE_TIME_ASYNC_CLASSIC_TASK_TURN_VISIBILITY_HTML)
}

pub(super) async fn parse_time_async_classic_post_parse_turns_page() -> Html<&'static str> {
    Html(PARSE_TIME_ASYNC_CLASSIC_POST_PARSE_TURNS_HTML)
}

pub(super) async fn parse_time_async_classic_post_parse_slow_second_page() -> Html<&'static str> {
    Html(PARSE_TIME_ASYNC_CLASSIC_POST_PARSE_SLOW_SECOND_HTML)
}

pub(super) async fn blocking_stylesheet_parser_blocking_external_page() -> Html<&'static str> {
    Html(BLOCKING_STYLESHEET_PARSER_BLOCKING_EXTERNAL_HTML)
}

pub(super) async fn blocking_stylesheet_parser_blocking_document_write_page() -> Html<&'static str>
{
    Html(BLOCKING_STYLESHEET_PARSER_BLOCKING_DOCUMENT_WRITE_HTML)
}

pub(super) async fn blocking_stylesheet_defer_page() -> Html<&'static str> {
    Html(BLOCKING_STYLESHEET_DEFER_HTML)
}

pub(super) async fn blocking_stylesheet_module_page() -> Html<&'static str> {
    Html(BLOCKING_STYLESHEET_MODULE_HTML)
}

pub(super) async fn phase_two_upgrade_runtime_style_defer_page() -> Html<&'static str> {
    Html(PHASE_TWO_UPGRADE_RUNTIME_STYLE_DEFER_HTML)
}

pub(super) async fn phase_two_upgrade_runtime_style_module_page() -> Html<&'static str> {
    Html(PHASE_TWO_UPGRADE_RUNTIME_STYLE_MODULE_HTML)
}

pub(super) async fn phase_two_shared_blocking_stylesheet_defer_page() -> Html<&'static str> {
    Html(PHASE_TWO_SHARED_BLOCKING_STYLESHEET_DEFER_HTML)
}

pub(super) async fn phase_two_shared_blocking_stylesheet_module_page() -> Html<&'static str> {
    Html(PHASE_TWO_SHARED_BLOCKING_STYLESHEET_MODULE_HTML)
}

pub(super) async fn blocking_stylesheet_parser_created_style_import_parser_blocking_external_page()
-> Html<&'static str> {
    Html(BLOCKING_STYLESHEET_PARSER_CREATED_STYLE_IMPORT_PARSER_BLOCKING_EXTERNAL_HTML)
}

pub(super) async fn blocking_stylesheet_parser_created_style_import_module_page()
-> Html<&'static str> {
    Html(BLOCKING_STYLESHEET_PARSER_CREATED_STYLE_IMPORT_MODULE_HTML)
}

pub(super) async fn blocking_stylesheet_alternate_non_blocking_page() -> Html<&'static str> {
    Html(BLOCKING_STYLESHEET_ALTERNATE_NON_BLOCKING_HTML)
}

pub(super) async fn document_write_implicit_replace_drops_old_defer_page() -> Html<&'static str> {
    Html(DOCUMENT_WRITE_IMPLICIT_REPLACE_DROPS_OLD_DEFER_HTML)
}

pub(super) async fn document_write_implicit_replace_drops_old_module_page() -> Html<&'static str> {
    Html(DOCUMENT_WRITE_IMPLICIT_REPLACE_DROPS_OLD_MODULE_HTML)
}

pub(super) async fn document_write_replacement_async_stays_after_domcontentloaded_page()
-> Html<&'static str> {
    Html(DOCUMENT_WRITE_REPLACEMENT_ASYNC_STAYS_AFTER_DOMCONTENTLOADED_HTML)
}

pub(super) async fn document_write_replacement_style_source_sync_page() -> Html<&'static str> {
    Html(DOCUMENT_WRITE_REPLACEMENT_STYLE_SOURCE_SYNC_HTML)
}

pub(super) async fn document_write_nested_writer_restores_outer_insertion_point_page()
-> Html<&'static str> {
    Html(DOCUMENT_WRITE_NESTED_WRITER_RESTORES_OUTER_INSERTION_POINT_HTML)
}

pub(super) async fn document_write_nested_external_script_serializes_outer_resume_page()
-> Html<&'static str> {
    Html(DOCUMENT_WRITE_NESTED_EXTERNAL_SCRIPT_SERIALIZES_OUTER_RESUME_HTML)
}

pub(super) async fn document_write_external_split_script_parser_session_page() -> Html<&'static str>
{
    Html(DOCUMENT_WRITE_EXTERNAL_SPLIT_SCRIPT_PARSER_SESSION_HTML)
}

pub(super) async fn document_write_inserted_external_resumes_chunked_root_page() -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel(2);
    tokio::spawn(async move {
        if tx
            .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from_static(
                DOCUMENT_WRITE_INSERTED_EXTERNAL_CHUNKED_HEAD.as_bytes(),
            )))
            .await
            .is_err()
        {
            return;
        }
        sleep(Duration::from_millis(5)).await;
        let _ = tx
            .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from_static(
                DOCUMENT_WRITE_INSERTED_EXTERNAL_CHUNKED_TAIL.as_bytes(),
            )))
            .await;
    });

    Response::builder()
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from_stream(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .expect("chunked document.write external-script response should build")
}

pub(super) async fn document_write_parser_visible_dom_boundary_page() -> Html<&'static str> {
    Html(DOCUMENT_WRITE_PARSER_VISIBLE_DOM_BOUNDARY_HTML)
}

pub(super) async fn document_write_external_parser_blocking_boundary_page() -> Html<&'static str> {
    Html(DOCUMENT_WRITE_EXTERNAL_PARSER_BLOCKING_BOUNDARY_HTML)
}

pub(super) async fn document_write_external_script_load_microtask_before_later_written_inline_page()
-> Html<&'static str> {
    Html(DOCUMENT_WRITE_EXTERNAL_SCRIPT_LOAD_MICROTASK_BEFORE_LATER_WRITTEN_INLINE_HTML)
}

pub(super) async fn document_write_importmap_before_written_module_page() -> Html<&'static str> {
    Html(DOCUMENT_WRITE_IMPORTMAP_BEFORE_WRITTEN_MODULE_HTML)
}

pub(super) async fn document_write_importmap_before_written_external_module_page()
-> Html<&'static str> {
    Html(DOCUMENT_WRITE_IMPORTMAP_BEFORE_WRITTEN_EXTERNAL_MODULE_HTML)
}

pub(super) async fn document_write_invalid_importmap_before_written_module_page()
-> Html<&'static str> {
    Html(DOCUMENT_WRITE_INVALID_IMPORTMAP_BEFORE_WRITTEN_MODULE_HTML)
}

pub(super) async fn document_write_invalid_importmap_before_restore_inline_page()
-> Html<&'static str> {
    Html(DOCUMENT_WRITE_INVALID_IMPORTMAP_BEFORE_RESTORE_INLINE_HTML)
}

pub(super) async fn document_write_defer_queues_after_later_classic_page() -> Html<&'static str> {
    Html(DOCUMENT_WRITE_DEFER_QUEUES_AFTER_LATER_CLASSIC_HTML)
}

pub(super) async fn document_write_defer_runs_before_domcontentloaded_page() -> Html<&'static str> {
    Html(DOCUMENT_WRITE_DEFER_RUNS_BEFORE_DOMCONTENTLOADED_HTML)
}

pub(super) async fn child_document_open_after_parent_load_data_script_page() -> Html<&'static str> {
    Html(CHILD_DOCUMENT_OPEN_AFTER_PARENT_LOAD_DATA_SCRIPT_HTML)
}

pub(super) async fn imported_started_child_script_stays_inert_page() -> Html<&'static str> {
    Html(IMPORTED_STARTED_CHILD_SCRIPT_STAYS_INERT_HTML)
}

pub(super) async fn document_open_after_load_external_scripts_page() -> Html<&'static str> {
    Html(DOCUMENT_OPEN_AFTER_LOAD_EXTERNAL_SCRIPTS_HTML)
}

pub(super) async fn document_write_multi_level_nested_writer_page() -> Html<&'static str> {
    Html(DOCUMENT_WRITE_MULTI_LEVEL_NESTED_WRITER_HTML)
}

pub(super) async fn document_write_late_stylesheet_does_not_block_written_module_page()
-> Html<&'static str> {
    Html(DOCUMENT_WRITE_LATE_STYLESHEET_DOES_NOT_BLOCK_WRITTEN_MODULE_HTML)
}

pub(super) async fn document_write_split_tags_stream_across_calls_page() -> Html<&'static str> {
    Html(DOCUMENT_WRITE_SPLIT_TAGS_STREAM_ACROSS_CALLS_HTML)
}

pub(super) async fn document_write_split_script_stream_across_calls_page() -> Html<&'static str> {
    Html(DOCUMENT_WRITE_SPLIT_SCRIPT_STREAM_ACROSS_CALLS_HTML)
}

pub(super) async fn document_write_split_external_script_stream_across_calls_page()
-> Html<&'static str> {
    Html(DOCUMENT_WRITE_SPLIT_EXTERNAL_SCRIPT_STREAM_ACROSS_CALLS_HTML)
}

pub(super) async fn document_write_split_importmap_and_module_stream_across_calls_page()
-> Html<&'static str> {
    Html(DOCUMENT_WRITE_SPLIT_IMPORTMAP_AND_MODULE_STREAM_ACROSS_CALLS_HTML)
}

pub(super) async fn runtime_inserted_stylesheet_load_page() -> Html<&'static str> {
    Html(RUNTIME_INSERTED_STYLESHEET_LOAD_HTML)
}

pub(super) async fn runtime_inserted_stylesheet_load_syncs_parser_snapshot_page()
-> Html<&'static str> {
    Html(RUNTIME_INSERTED_STYLESHEET_LOAD_SYNCS_PARSER_SNAPSHOT_HTML)
}

pub(super) async fn runtime_inserted_stylesheet_load_triggers_location_replace_page()
-> Html<&'static str> {
    Html(RUNTIME_INSERTED_STYLESHEET_LOAD_TRIGGERS_LOCATION_REPLACE_HTML)
}

pub(super) async fn runtime_inserted_stylesheet_href_mutation_uses_fresh_fetch_page()
-> Html<&'static str> {
    Html(RUNTIME_INSERTED_STYLESHEET_HREF_MUTATION_USES_FRESH_FETCH_HTML)
}

pub(super) async fn runtime_inserted_style_import_missing_completes_load_page() -> Html<&'static str>
{
    Html(RUNTIME_INSERTED_STYLE_IMPORT_MISSING_COMPLETES_LOAD_HTML)
}

pub(super) async fn dynamic_script_waits_for_runtime_inserted_stylesheet_page() -> Html<&'static str>
{
    Html(DYNAMIC_SCRIPT_WAITS_FOR_RUNTIME_INSERTED_STYLESHEET_HTML)
}

pub(super) async fn runtime_inserted_preload_and_modulepreload_parser_progress_page()
-> Html<&'static str> {
    Html(RUNTIME_INSERTED_PRELOAD_AND_MODULEPRELOAD_PARSER_PROGRESS_HTML)
}

pub(super) async fn modulepreload_shared_static_dependency_page() -> Html<&'static str> {
    Html(MODULEPRELOAD_SHARED_STATIC_DEPENDENCY_HTML)
}

pub(super) async fn modulepreload_duplicate_shared_static_dependency_page() -> Html<&'static str> {
    Html(MODULEPRELOAD_DUPLICATE_SHARED_STATIC_DEPENDENCY_HTML)
}

pub(super) async fn duplicate_module_root_eval_page() -> Html<&'static str> {
    Html(DUPLICATE_MODULE_ROOT_EVAL_HTML)
}

pub(super) async fn duplicate_module_root_with_nested_dependencies_page() -> Html<&'static str> {
    Html(DUPLICATE_MODULE_ROOT_WITH_NESTED_DEPENDENCIES_HTML)
}

pub(super) async fn module_top_level_fetch_and_mime_errors_page() -> Html<&'static str> {
    Html(MODULE_TOP_LEVEL_FETCH_AND_MIME_ERRORS_HTML)
}

pub(super) async fn modulepreload_reused_parent_pending_child_page() -> Html<&'static str> {
    Html(MODULEPRELOAD_REUSED_PARENT_PENDING_CHILD_HTML)
}

pub(super) async fn declarative_shadow_adopted_stylesheets_modulepreload_page() -> Html<&'static str>
{
    Html(
        r#"<!doctype html>
<html>
<head><title>shadow adoptedStyleSheets modulepreload</title></head>
<body>
<script>
window.shadowModulepreloadResult = "pending";
const cssUrl = "/assets/shadow_adopted_modulepreload.css";
const link = document.createElement("link");
link.rel = "modulepreload";
link.as = "style";
link.href = cssUrl;
link.onload = () => {
  const wrapper = document.createElement("section");
  wrapper.setHTMLUnsafe(
    "<div id='host1'>" +
      "<template shadowrootmode='open' shadowrootadoptedstylesheets='" + cssUrl + "'>" +
        "<span id='test1'>Test 1</span>" +
      "</template>" +
    "</div>" +
    "<div id='host2'>" +
      "<template shadowrootmode='open' shadowrootadoptedstylesheets='" + cssUrl + "'>" +
        "<span id='test2'>Test 2</span>" +
      "</template>" +
    "</div>"
  );
  document.body.appendChild(wrapper);
  const root1 = document.getElementById("host1").shadowRoot;
  const root2 = document.getElementById("host2").shadowRoot;
  window.shadowModulepreloadResult = [
    root1.adoptedStyleSheets.length,
    root2.adoptedStyleSheets.length,
    root1.adoptedStyleSheets[0].cssRules.length,
    root1.adoptedStyleSheets[0].cssRules[0].cssText,
    root1.adoptedStyleSheets[0] === root2.adoptedStyleSheets[0],
    getComputedStyle(root1.getElementById("test1")).color,
    getComputedStyle(root2.getElementById("test2")).color
  ].join("|");
};
link.onerror = () => {
  window.shadowModulepreloadResult = "error";
};
document.head.appendChild(link);
</script>
</body>
</html>"#,
    )
}

pub(super) async fn declarative_shadow_adopted_stylesheets_dynamic_import_page()
-> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
<head><title>shadow adoptedStyleSheets dynamic CSS import</title></head>
<body>
<script>
window.shadowCssImportResult = "pending";
const cssUrl = "/assets/shadow_adopted_modulepreload.css";
document.body.setHTMLUnsafe(
  "<div id='host'>" +
    "<template shadowrootmode='open' shadowrootadoptedstylesheets='" + cssUrl + "'>" +
      "<span id='test'>Test</span>" +
    "</template>" +
  "</div>"
);
const root = document.getElementById("host").shadowRoot;
const placeholder = root.adoptedStyleSheets[0];
import(cssUrl, { with: { type: "css" } }).then((module) => {
  const sheet = root.adoptedStyleSheets[0];
  window.shadowCssImportResult = [
    placeholder === sheet,
    module.default === sheet,
    sheet.cssRules.length,
    sheet.cssRules[0].cssText,
    getComputedStyle(root.getElementById("test")).color
  ].join("|");
}, (error) => {
  window.shadowCssImportResult = "error:" + error.message;
});
</script>
</body>
</html>"#,
    )
}

pub(super) async fn dynamic_script_async_overtakes_in_order_page() -> Html<&'static str> {
    Html(DYNAMIC_SCRIPT_ASYNC_OVERTAKES_IN_ORDER_HTML)
}

pub(super) async fn dynamic_script_in_order_preserves_order_page() -> Html<&'static str> {
    Html(DYNAMIC_SCRIPT_IN_ORDER_PRESERVES_ORDER_HTML)
}

pub(super) async fn parse_time_dynamic_script_load_after_parser_progress_page() -> Html<&'static str>
{
    Html(PARSE_TIME_DYNAMIC_SCRIPT_LOAD_AFTER_PARSER_PROGRESS_HTML)
}

pub(super) async fn parse_time_dynamic_script_error_after_parser_progress_page()
-> Html<&'static str> {
    Html(PARSE_TIME_DYNAMIC_SCRIPT_ERROR_AFTER_PARSER_PROGRESS_HTML)
}

pub(super) async fn parser_connected_external_classic_dispatches_load_page() -> Html<&'static str> {
    Html(PARSER_CONNECTED_EXTERNAL_CLASSIC_DISPATCHES_LOAD_HTML)
}

pub(super) async fn parser_connected_external_classic_load_document_write_insertion_point_page()
-> Html<&'static str> {
    Html(PARSER_CONNECTED_EXTERNAL_CLASSIC_LOAD_DOCUMENT_WRITE_INSERTION_POINT_HTML)
}

pub(super) async fn parser_connected_external_classic_load_document_write_parent_callback_page()
-> Html<&'static str> {
    Html(PARSER_CONNECTED_EXTERNAL_CLASSIC_LOAD_DOCUMENT_WRITE_PARENT_CALLBACK_HTML)
}

pub(super) async fn parser_connected_external_classic_load_document_write_parent_callback_child_page()
-> Html<&'static str> {
    Html(PARSER_CONNECTED_EXTERNAL_CLASSIC_LOAD_DOCUMENT_WRITE_PARENT_CALLBACK_CHILD_HTML)
}

pub(super) async fn parser_connected_external_classic_error_microtask_page() -> Html<&'static str> {
    Html(PARSER_CONNECTED_EXTERNAL_CLASSIC_ERROR_MICROTASK_HTML)
}

pub(super) async fn parser_connected_external_classic_unknown_scheme_errors_and_continues_page()
-> Html<&'static str> {
    Html(PARSER_CONNECTED_EXTERNAL_CLASSIC_UNKNOWN_SCHEME_ERRORS_AND_CONTINUES_HTML)
}

pub(super) async fn parser_connected_inline_classic_does_not_dispatch_load_page()
-> Html<&'static str> {
    Html(PARSER_CONNECTED_INLINE_CLASSIC_DOES_NOT_DISPATCH_LOAD_HTML)
}

pub(super) async fn parser_owned_external_defer_dispatches_load_page() -> Html<&'static str> {
    Html(PARSER_OWNED_EXTERNAL_DEFER_DISPATCHES_LOAD_HTML)
}

pub(super) async fn parser_owned_external_async_dispatches_load_page() -> Html<&'static str> {
    Html(PARSER_OWNED_EXTERNAL_ASYNC_DISPATCHES_LOAD_HTML)
}

pub(super) async fn runtime_owned_external_in_order_load_after_domcontentloaded_page()
-> Html<&'static str> {
    Html(RUNTIME_OWNED_EXTERNAL_IN_ORDER_LOAD_AFTER_DOMCONTENTLOADED_HTML)
}

pub(super) async fn runtime_owned_external_in_order_with_defer_stays_after_domcontentloaded_page()
-> Html<&'static str> {
    Html(RUNTIME_OWNED_EXTERNAL_IN_ORDER_WITH_DEFER_STAYS_AFTER_DOMCONTENTLOADED_HTML)
}

pub(super) async fn runtime_owned_external_in_order_error_after_domcontentloaded_page()
-> Html<&'static str> {
    Html(RUNTIME_OWNED_EXTERNAL_IN_ORDER_ERROR_AFTER_DOMCONTENTLOADED_HTML)
}

pub(super) async fn release_runtime_owned_external_in_order_error_after_domcontentloaded(
    headers: HeaderMap,
) -> StatusCode {
    let host_key = request_host_key(&headers).unwrap_or_default();
    notify_runtime_owned_in_order_error_after_dcl_gate(&host_key);
    StatusCode::NO_CONTENT
}

pub(super) async fn runtime_owned_external_in_order_from_domcontentloaded_handler_page()
-> Html<&'static str> {
    Html(RUNTIME_OWNED_EXTERNAL_IN_ORDER_FROM_DOMCONTENTLOADED_HANDLER_HTML)
}

pub(super) async fn runtime_owned_external_async_does_not_block_domcontentloaded_page()
-> Html<&'static str> {
    Html(RUNTIME_OWNED_EXTERNAL_ASYNC_DOES_NOT_BLOCK_DOMCONTENTLOADED_HTML)
}

pub(super) async fn runtime_owned_external_async_fast_does_not_overtake_domcontentloaded_page()
-> Html<&'static str> {
    Html(RUNTIME_OWNED_EXTERNAL_ASYNC_FAST_DOES_NOT_OVERTAKE_DOMCONTENTLOADED_HTML)
}

pub(super) async fn runtime_owned_external_async_fast_streaming_tail_page(
    headers: HeaderMap,
) -> Response {
    let split_marker = "<main id=\"late\">";
    let split_index = RUNTIME_OWNED_EXTERNAL_ASYNC_FAST_DOES_NOT_OVERTAKE_DOMCONTENTLOADED_HTML
        .find(split_marker)
        .expect("streaming runtime-owned async fixture must include late DOM marker");
    let (head, tail) = RUNTIME_OWNED_EXTERNAL_ASYNC_FAST_DOES_NOT_OVERTAKE_DOMCONTENTLOADED_HTML
        .split_at(split_index);
    let head = head.as_bytes().to_vec();
    let tail = tail.as_bytes().to_vec();
    let host_key = request_host_key(&headers).unwrap_or_default();
    let tail_gate = runtime_owned_async_chunked_tail_gate(&host_key);
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    tokio::spawn(async move {
        if tx
            .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from(head)))
            .await
            .is_err()
        {
            return;
        }
        let _ = tokio::time::timeout(Duration::from_millis(500), tail_gate.notified()).await;
        // Keep the parser tail unavailable until the already-requested script
        // can complete. Chromium then permits this dynamic async task while
        // `document.readyState` is still `loading`.
        sleep(Duration::from_millis(20)).await;
        let _ = tx
            .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from(tail)))
            .await;
        if !host_key.is_empty() {
            remove_runtime_owned_async_chunked_tail_gate(&host_key);
        }
    });

    Response::builder()
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from_stream(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .expect("streaming runtime-owned async response should build")
}

pub(super) async fn runtime_owned_external_async_with_defer_does_not_block_domcontentloaded_page()
-> Html<&'static str> {
    Html(RUNTIME_OWNED_EXTERNAL_ASYNC_WITH_DEFER_DOES_NOT_BLOCK_DOMCONTENTLOADED_HTML)
}

pub(super) async fn runtime_owned_default_async_module_side_effect_after_domcontentloaded_page()
-> Html<&'static str> {
    Html(RUNTIME_OWNED_DEFAULT_ASYNC_MODULE_SIDE_EFFECT_AFTER_DOMCONTENTLOADED_HTML)
}

pub(super) async fn runtime_owned_inline_module_single_line_import_executes_page()
-> Html<&'static str> {
    Html(RUNTIME_OWNED_INLINE_MODULE_SINGLE_LINE_IMPORT_EXECUTES_HTML)
}

pub(super) async fn runtime_owned_inline_module_runs_while_parser_defer_is_blocked_page()
-> Html<&'static str> {
    Html(RUNTIME_OWNED_INLINE_MODULE_RUNS_WHILE_PARSER_DEFER_IS_BLOCKED_HTML)
}

pub(super) async fn runtime_owned_inline_module_missing_default_export_after_domcontentloaded_page()
-> Html<&'static str> {
    Html(RUNTIME_OWNED_INLINE_MODULE_MISSING_DEFAULT_EXPORT_AFTER_DOMCONTENTLOADED_HTML)
}

pub(super) async fn runtime_owned_external_module_load_failure_after_later_module_page()
-> Html<&'static str> {
    Html(RUNTIME_OWNED_EXTERNAL_MODULE_LOAD_FAILURE_AFTER_LATER_MODULE_HTML)
}

pub(super) async fn runtime_inserted_inline_script_does_not_dispatch_load_page()
-> Html<&'static str> {
    Html(RUNTIME_INSERTED_INLINE_SCRIPT_DOES_NOT_DISPATCH_LOAD_HTML)
}

pub(super) async fn document_write_external_script_load_after_page_task_turn_page()
-> Html<&'static str> {
    Html(DOCUMENT_WRITE_EXTERNAL_SCRIPT_LOAD_AFTER_PAGE_TASK_TURN_HTML)
}

pub(super) async fn document_write_external_script_error_after_page_task_turn_page()
-> Html<&'static str> {
    Html(DOCUMENT_WRITE_EXTERNAL_SCRIPT_ERROR_AFTER_PAGE_TASK_TURN_HTML)
}

pub(super) async fn document_write_delayed_external_script_does_not_block_parent_runtime_page()
-> Html<&'static str> {
    Html(DOCUMENT_WRITE_DELAYED_EXTERNAL_SCRIPT_DOES_NOT_BLOCK_PARENT_RUNTIME_HTML)
}

pub(super) async fn document_open_during_parser_script_with_pending_written_external_is_ignored_page()
-> Html<&'static str> {
    Html(DOCUMENT_OPEN_DURING_PARSER_SCRIPT_WITH_PENDING_WRITTEN_EXTERNAL_IS_IGNORED_HTML)
}

pub(super) async fn dynamic_script_type_mutation_remains_inert_page() -> Html<&'static str> {
    Html(DYNAMIC_SCRIPT_TYPE_MUTATION_REMAINS_INERT_HTML)
}

pub(super) async fn dynamic_script_reattach_stays_started_page() -> Html<&'static str> {
    Html(DYNAMIC_SCRIPT_REATTACH_STAYS_STARTED_HTML)
}

pub(super) async fn dynamic_script_src_mutation_stays_started_page() -> Html<&'static str> {
    Html(DYNAMIC_SCRIPT_SRC_MUTATION_STAYS_STARTED_HTML)
}

pub(super) async fn dynamic_script_async_attr_clears_force_async_page() -> Html<&'static str> {
    Html(DYNAMIC_SCRIPT_ASYNC_ATTR_CLEARS_FORCE_ASYNC_HTML)
}

pub(super) async fn dynamic_script_src_added_after_connect_starts_once_page() -> Html<&'static str>
{
    Html(DYNAMIC_SCRIPT_SRC_ADDED_AFTER_CONNECT_STARTS_ONCE_HTML)
}

pub(super) async fn dynamic_script_error_does_not_abort_queue_page() -> Html<&'static str> {
    Html(DYNAMIC_SCRIPT_ERROR_DOES_NOT_ABORT_QUEUE_HTML)
}

pub(super) async fn dynamic_script_preparation_context_stays_in_old_document_page()
-> Html<&'static str> {
    Html(DYNAMIC_SCRIPT_PREPARATION_CONTEXT_STAYS_IN_OLD_DOCUMENT_HTML)
}

pub(super) async fn dynamic_importmap_before_module_page() -> Html<&'static str> {
    Html(DYNAMIC_IMPORTMAP_BEFORE_MODULE_HTML)
}

pub(super) async fn dynamic_async_module_closes_importmap_acquisition_page() -> Html<&'static str> {
    Html(DYNAMIC_ASYNC_MODULE_CLOSES_IMPORTMAP_ACQUISITION_HTML)
}

pub(super) async fn dynamic_external_importmap_error_before_module_page() -> Html<&'static str> {
    Html(DYNAMIC_EXTERNAL_IMPORTMAP_ERROR_BEFORE_MODULE_HTML)
}

pub(super) async fn dynamic_module_execution_failure_does_not_abort_queue_page()
-> Html<&'static str> {
    Html(DYNAMIC_MODULE_EXECUTION_FAILURE_DOES_NOT_ABORT_QUEUE_HTML)
}

pub(super) async fn dynamic_module_pending_star_missing_export_does_not_abort_queue_page()
-> Html<&'static str> {
    Html(DYNAMIC_MODULE_PENDING_STAR_MISSING_EXPORT_DOES_NOT_ABORT_QUEUE_HTML)
}

pub(super) async fn dynamic_module_pending_star_link_failure_before_body_and_later_module_page()
-> Html<&'static str> {
    Html(DYNAMIC_MODULE_PENDING_STAR_LINK_FAILURE_BEFORE_BODY_AND_LATER_MODULE_HTML)
}

pub(super) async fn dynamic_module_pending_star_final_missing_reports_link_failure_page()
-> Html<&'static str> {
    Html(DYNAMIC_MODULE_PENDING_STAR_FINAL_MISSING_REPORTS_LINK_FAILURE_HTML)
}

pub(super) async fn dynamic_module_tla_rejection_does_not_abort_queue_page() -> Html<&'static str> {
    Html(DYNAMIC_MODULE_TLA_REJECTION_DOES_NOT_ABORT_QUEUE_HTML)
}

pub(super) async fn importmap_scopes_and_prefixes_page() -> Html<&'static str> {
    Html(IMPORTMAP_SCOPES_AND_PREFIXES_HTML)
}

pub(super) async fn importmap_merge_after_resolution_page() -> Html<&'static str> {
    Html(IMPORTMAP_MERGE_AFTER_RESOLUTION_HTML)
}

pub(super) async fn importmap_url_like_normalization_page() -> Html<&'static str> {
    Html(IMPORTMAP_URL_LIKE_NORMALIZATION_HTML)
}

pub(super) async fn importmap_after_module_load_page() -> Html<&'static str> {
    Html(IMPORTMAP_AFTER_MODULE_LOAD_HTML)
}

pub(super) async fn importmap_closed_by_parser_owned_module_before_late_dynamic_map_page()
-> Html<&'static str> {
    Html(IMPORTMAP_CLOSED_BY_PARSER_OWNED_MODULE_BEFORE_LATE_DYNAMIC_MAP_HTML)
}

pub(super) async fn parser_owned_importmap_blocked_after_dynamic_module_prepare_page()
-> Html<&'static str> {
    Html(PARSER_OWNED_IMPORTMAP_BLOCKED_AFTER_DYNAMIC_MODULE_PREPARE_HTML)
}

pub(super) async fn importmap_null_blocks_dynamic_import_page() -> Html<&'static str> {
    Html(IMPORTMAP_NULL_BLOCKS_DYNAMIC_IMPORT_HTML)
}

pub(super) async fn module_bare_specifier_without_importmap_page() -> Html<&'static str> {
    Html(MODULE_BARE_SPECIFIER_WITHOUT_IMPORTMAP_HTML)
}

pub(super) async fn module_default_and_side_effect_imports_page() -> Html<&'static str> {
    Html(MODULE_DEFAULT_AND_SIDE_EFFECT_IMPORTS_HTML)
}

pub(super) async fn module_default_reexport_page() -> Html<&'static str> {
    Html(MODULE_DEFAULT_REEXPORT_HTML)
}

pub(super) async fn module_string_literal_export_names_page() -> Html<&'static str> {
    Html(MODULE_STRING_LITERAL_EXPORT_NAMES_HTML)
}

pub(super) async fn module_string_literal_export_names_surrogate_pairs_page() -> Html<&'static str>
{
    Html(MODULE_STRING_LITERAL_EXPORT_NAMES_SURROGATE_PAIRS_HTML)
}

pub(super) async fn module_export_star_string_literal_namespace_page() -> Html<&'static str> {
    Html(MODULE_EXPORT_STAR_STRING_LITERAL_NAMESPACE_HTML)
}

pub(super) async fn module_escaped_identifier_names_page() -> Html<&'static str> {
    Html(MODULE_ESCAPED_IDENTIFIER_NAMES_HTML)
}

pub(super) async fn module_export_default_function_and_class_page() -> Html<&'static str> {
    Html(MODULE_EXPORT_DEFAULT_FUNCTION_AND_CLASS_HTML)
}

pub(super) async fn module_export_default_anonymous_declarations_page() -> Html<&'static str> {
    Html(MODULE_EXPORT_DEFAULT_ANONYMOUS_DECLARATIONS_HTML)
}

pub(super) async fn module_export_class_named_page() -> Html<&'static str> {
    Html(MODULE_EXPORT_CLASS_NAMED_HTML)
}

pub(super) async fn module_export_generator_functions_page() -> Html<&'static str> {
    Html(MODULE_EXPORT_GENERATOR_FUNCTIONS_HTML)
}

pub(super) async fn module_export_const_multiple_bindings_page() -> Html<&'static str> {
    Html(MODULE_EXPORT_CONST_MULTIPLE_BINDINGS_HTML)
}

pub(super) async fn parser_module_completion_dcl_before_timer_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script>
      globalThis.parserModuleCompletionOrder = [];
      document.addEventListener("DOMContentLoaded", () => {
        globalThis.parserModuleCompletionOrder.push("dcl");
      });
    </script>
    <script type="module">
      globalThis.parserModuleCompletionOrder.push("module");
      setTimeout(() => {
        globalThis.parserModuleCompletionOrder.push("timer");
        globalThis.parserModuleCompletionFinalOrder =
          globalThis.parserModuleCompletionOrder.join(",");
      }, 0);
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_export_destructuring_bindings_page() -> Html<&'static str> {
    Html(MODULE_EXPORT_DESTRUCTURING_BINDINGS_HTML)
}

pub(super) async fn module_export_nested_destructuring_bindings_page() -> Html<&'static str> {
    Html(MODULE_EXPORT_NESTED_DESTRUCTURING_BINDINGS_HTML)
}

pub(super) async fn module_export_nested_initializer_commas_page() -> Html<&'static str> {
    Html(MODULE_EXPORT_NESTED_INITIALIZER_COMMAS_HTML)
}

pub(super) async fn module_import_export_list_comments_page() -> Html<&'static str> {
    Html(MODULE_IMPORT_EXPORT_LIST_COMMENTS_HTML)
}

pub(super) async fn module_multiline_dynamic_import_page() -> Html<&'static str> {
    Html(MODULE_MULTILINE_DYNAMIC_IMPORT_HTML)
}

pub(super) async fn module_dynamic_import_comments_and_trailing_comma_page() -> Html<&'static str> {
    Html(MODULE_DYNAMIC_IMPORT_COMMENTS_AND_TRAILING_COMMA_HTML)
}

pub(super) async fn module_dynamic_import_static_concat_page() -> Html<&'static str> {
    Html(MODULE_DYNAMIC_IMPORT_STATIC_CONCAT_HTML)
}

pub(super) async fn module_dynamic_import_source_rejects_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script type="module">
      try {
        await import.source("/assets/module-dynamic-import-comments-target.mjs");
        window.moduleDynamicImportSourceRejected = false;
      } catch (error) {
        window.moduleDynamicImportSourceRejected = true;
        window.moduleDynamicImportSourceErrorName = error && error.name;
        window.moduleDynamicImportSourceMessageIncludesSourcePhase =
          String(error && error.message).includes("source-phase");
      }
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn dynamic_import_document_write_iframe_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script>
      window.dynamicImportDocumentWriteDone = false;
    </script>
    <iframe src="/compat/dynamic-import-document-write-child"></iframe>
  </body>
</html>"#,
    )
}

pub(super) async fn dynamic_import_document_write_child_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<script type="module">
  (async () => {
    try {
      await import("/assets/dynamic-import-document-write-target.mjs");
      parent.dynamicImportDocumentWriteResolved = true;
    } catch (error) {
      parent.dynamicImportDocumentWriteRejected = String(error && error.message || error);
    }
  })();
</script>
Initial body contents
"#,
    )
}

pub(super) async fn module_import_attributes_and_dynamic_options_page() -> Html<&'static str> {
    Html(MODULE_IMPORT_ATTRIBUTES_AND_DYNAMIC_OPTIONS_HTML)
}

pub(super) async fn module_import_assertions_legacy_syntax_page() -> Html<&'static str> {
    Html(MODULE_IMPORT_ASSERTIONS_LEGACY_SYNTAX_HTML)
}

pub(super) async fn module_static_json_css_import_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script>
      window.moduleStaticJsonCssOrder = [];
      document.addEventListener("DOMContentLoaded", () => {
        window.moduleStaticJsonCssOrder.push("dcl");
        window.moduleStaticJsonCssFinalOrder =
          window.moduleStaticJsonCssOrder.join(",");
      });
    </script>
    <script type="module">
      import data from "/assets/module-synthetic-data.json" with { type: "json" };
      import sheet from "/assets/module-synthetic-style.css" with { type: "css" };

      window.moduleStaticJsonCssValue = data.answer + ":" + data.label;
      window.moduleStaticJsonCssSheetRules = sheet.cssRules.length;
      window.moduleStaticJsonCssSheetText = sheet.cssRules[0].cssText;
      window.moduleStaticJsonCssOrder.push("module");
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_static_wasm_import_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script type="module">
      import * as mod from "/assets/module-wasm-exported-names.wasm";
      import { glob as namedGlob } from "/assets/module-wasm-exported-names.wasm";

      window.moduleStaticWasmExports =
        Object.getOwnPropertyNames(mod).sort().join(",");
      window.moduleStaticWasmFuncType = typeof mod.func;
      window.moduleStaticWasmMem = mod.mem instanceof WebAssembly.Memory;
      window.moduleStaticWasmGlob = mod.glob instanceof WebAssembly.Global;
      window.moduleStaticWasmGlobValue = mod.glob;
      window.moduleStaticWasmNamedGlob =
        namedGlob instanceof WebAssembly.Global;
      window.moduleStaticWasmNamedGlobValue = namedGlob;
      window.moduleStaticWasmTab = mod.tab instanceof WebAssembly.Table;
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_dynamic_wasm_import_ignores_patched_instance_page() -> Html<&'static str>
{
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script type="module">
      window.moduleDynamicWasmPatchedInstanceDone = false;
      window.moduleDynamicWasmPatchedInstanceError = "";
      const originalInstance = WebAssembly.Instance;
      WebAssembly.Instance = function PatchedWebAssemblyInstance() {
        throw new Error("patched instance constructor");
      };

      try {
        const mod = await import("/assets/module-wasm-exported-names.wasm");
        window.moduleDynamicWasmPatchedInstanceFuncType = typeof mod.func;
        window.moduleDynamicWasmPatchedInstanceMem =
          mod.mem instanceof WebAssembly.Memory;
        window.moduleDynamicWasmPatchedInstanceGlob =
          mod.glob instanceof WebAssembly.Global;
        window.moduleDynamicWasmPatchedInstanceGlobValue = mod.glob;
        window.moduleDynamicWasmPatchedInstanceStillPatched =
          WebAssembly.Instance !== originalInstance;
        window.moduleDynamicWasmPatchedInstanceDone = true;
      } catch (error) {
        window.moduleDynamicWasmPatchedInstanceError =
          error && error.message ? error.message : String(error);
      }
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_wasm_namespace_instance_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script type="module">
      import * as staticNamespace from "/assets/module-wasm-exported-names.wasm";

      try {
        const staticInstance = WebAssembly.namespaceInstance(staticNamespace);
        const dynamicNamespace =
          await import("/assets/module-wasm-exported-names.wasm");
        const dynamicInstance =
          WebAssembly.namespaceInstance(dynamicNamespace);
        const dynamicNamespace2 =
          await import("/assets/module-wasm-exported-names.wasm");
        const dynamicInstance2 =
          WebAssembly.namespaceInstance(dynamicNamespace2);
        const jsNamespace =
          await import("/assets/module-source-phase-identity.js");

        let plainObjectRejected = false;
        let jsNamespaceRejected = false;
        try {
          WebAssembly.namespaceInstance({});
        } catch (error) {
          plainObjectRejected = error instanceof TypeError;
        }
        try {
          WebAssembly.namespaceInstance(jsNamespace);
        } catch (error) {
          jsNamespaceRejected = error instanceof TypeError;
        }

        window.moduleWasmNamespaceInstanceStatic =
          staticInstance instanceof WebAssembly.Instance;
        window.moduleWasmNamespaceInstanceShared =
          staticInstance === dynamicInstance &&
          dynamicInstance === dynamicInstance2;
        window.moduleWasmNamespaceInstanceFuncType =
          typeof dynamicInstance.exports.func;
        window.moduleWasmNamespaceInstanceRejectsPlainObject =
          plainObjectRejected;
        window.moduleWasmNamespaceInstanceRejectsJsNamespace =
          jsNamespaceRejected;
        window.moduleWasmNamespaceInstanceDone = true;
      } catch (error) {
        window.moduleWasmNamespaceInstanceError =
          error && error.constructor
            ? error.constructor.name + ":" + error.message
            : String(error);
      }
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_static_wasm_import_chain_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script type="module">
      window.moduleStaticWasmImportLog = [];
      import { logExec } from "/assets/wasm-import-from-wasm.wasm";

      logExec();
      window.moduleStaticWasmImportResult =
        window.moduleStaticWasmImportLog.join(",");
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_static_wasm_import_js_dependency_graph_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script type="module">
      window.moduleStaticWasmImportJsDependencyLog = [];
      import { log } from "/assets/wasm-import-js-dependency.wasm";

      log();
      window.moduleStaticWasmImportJsDependencyResult =
        window.moduleStaticWasmImportJsDependencyLog.join(",");
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_static_wasm_import_throwing_js_dependency_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script>
      window.moduleStaticWasmThrowingJsDependencyConstructor = "";
      window.moduleStaticWasmThrowingJsDependencyMessage = "";
      window.moduleStaticWasmThrowingJsDependencyScriptLoad = false;
      window.moduleStaticWasmThrowingJsDependencyScriptError = false;
      window.addEventListener("error", ev => {
        window.moduleStaticWasmThrowingJsDependencyConstructor =
          ev.error && ev.error.constructor ? ev.error.constructor.name : "";
        window.moduleStaticWasmThrowingJsDependencyMessage = ev.message;
      });
    </script>
    <script
      type="module"
      src="/assets/wasm-import-throwing-js-dependency-entry.js"
      onload="window.moduleStaticWasmThrowingJsDependencyScriptLoad = true"
      onerror="window.moduleStaticWasmThrowingJsDependencyScriptError = true"></script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_mutable_wasm_global_initial_value_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script type="module">
      import * as mod from "/assets/mutable-global.wasm";

      window.moduleMutableWasmGlobalType = typeof mod.glob;
      window.moduleMutableWasmGlobalIsGlobal =
        mod.glob instanceof WebAssembly.Global;
      window.moduleMutableWasmGlobalInitial = mod.glob;
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_wasm_global_unwrap_patched_getter_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script>
      window.moduleWasmGlobalPatchedGetterDone = false;
      window.moduleWasmGlobalPatchedGetterError = "";
      window.addEventListener("error", (event) => {
        window.moduleWasmGlobalPatchedGetterError =
          event.error && event.error.message
            ? event.error.message
            : event.message;
      });
      window.addEventListener("unhandledrejection", (event) => {
        const reason = event.reason;
        window.moduleWasmGlobalPatchedGetterError =
          reason && reason.message ? reason.message : String(reason);
      });

      const descriptor =
        Object.getOwnPropertyDescriptor(WebAssembly.Global.prototype, "value");
      Object.defineProperty(WebAssembly.Global.prototype, "value", {
        configurable: true,
        get() {
          throw new Error("patched WebAssembly.Global getter was used");
        },
        set: descriptor.set,
      });
    </script>
    <script type="module">
      import * as mod from "/assets/mutable-global.wasm";
      import { glob as namedGlob } from "/assets/mutable-global.wasm";

      window.moduleWasmGlobalPatchedGetterType = typeof mod.glob;
      window.moduleWasmGlobalPatchedGetterIsGlobal =
        mod.glob instanceof WebAssembly.Global;
      window.moduleWasmGlobalPatchedGetterValue = mod.glob;
      window.moduleWasmGlobalPatchedGetterNamedType = typeof namedGlob;
      window.moduleWasmGlobalPatchedGetterNamedIsGlobal =
        namedGlob instanceof WebAssembly.Global;
      window.moduleWasmGlobalPatchedGetterNamedValue = namedGlob;
      window.moduleWasmGlobalPatchedGetterDone = true;
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_mutable_wasm_global_live_binding_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script type="module">
      import * as mod from "/assets/mutable-global-with-v128.wasm";
      import {
        getGlobal,
        mutableValue,
        setGlobal,
      } from "/assets/mutable-global-with-v128.wasm";

      window.moduleMutableWasmGlobalLiveInitialNamespace = mod.mutableValue;
      window.moduleMutableWasmGlobalLiveInitialNamed = mutableValue;
      window.moduleMutableWasmGlobalLiveInitialGetter = getGlobal();

      setGlobal(555);

      window.moduleMutableWasmGlobalLiveGetterAfterSet = getGlobal();
      window.moduleMutableWasmGlobalLiveNamespaceAfterSet = mod.mutableValue;
      window.moduleMutableWasmGlobalLiveNamedAfterSet = mutableValue;
      window.moduleMutableWasmGlobalLiveType = typeof mod.mutableValue;
      window.moduleMutableWasmGlobalLiveIsGlobal =
        mod.mutableValue instanceof WebAssembly.Global;
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_wasm_global_dep_reexport_live_binding_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script type="module">
      import * as mod from "/assets/mutable-global-reexport.wasm";
      import {
        getImportedGlobal,
        reexportedMutableValue,
        setImportedGlobal,
      } from "/assets/mutable-global-reexport.wasm";

      window.moduleMutableWasmGlobalReexportInitialNamespace =
        mod.reexportedMutableValue;
      window.moduleMutableWasmGlobalReexportInitialNamed =
        reexportedMutableValue;
      window.moduleMutableWasmGlobalReexportInitialGetter =
        getImportedGlobal();

      setImportedGlobal(777);

      window.moduleMutableWasmGlobalReexportGetterAfterSet =
        getImportedGlobal();
      window.moduleMutableWasmGlobalReexportNamespaceAfterSet =
        mod.reexportedMutableValue;
      window.moduleMutableWasmGlobalReexportNamedAfterSet =
        reexportedMutableValue;
      window.moduleMutableWasmGlobalReexportType =
        typeof mod.reexportedMutableValue;
      window.moduleMutableWasmGlobalReexportIsGlobal =
        mod.reexportedMutableValue instanceof WebAssembly.Global;
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_source_phase_wasm_import_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script type="module">
      window.moduleSourcePhaseUnhandled = "";
      window.addEventListener("unhandledrejection", ev => {
        window.moduleSourcePhaseUnhandled =
          ev.reason && ev.reason.constructor
            ? ev.reason.constructor.name + ":" + ev.reason.message
            : String(ev.reason);
      });
      import source staticSource from "/assets/module-wasm-exported-names.wasm";
      import source sharedSource from "/assets/wasm-import-from-wasm.wasm";
      import { logExec as sharedLogExec } from "/assets/wasm-import-from-wasm.wasm";

      const AbstractModuleSource = Object.getPrototypeOf(WebAssembly.Module);
      const AbstractModuleSourceProto =
        Object.getPrototypeOf(WebAssembly.Module.prototype);
      window.moduleSourcePhaseAbstractModuleSourceHidden =
        !("AbstractModuleSource" in globalThis);
      window.moduleSourcePhaseAbstractModuleSourceName =
        AbstractModuleSource.name;
      window.moduleSourcePhaseModuleConstructorExtendsAbstract =
        AbstractModuleSource !== Function;
      window.moduleSourcePhaseModulePrototypeExtendsAbstract =
        AbstractModuleSource.prototype === AbstractModuleSourceProto;
      window.moduleStaticWasmImportLog = [];
      window.moduleSourcePhaseStaticIsModule =
        staticSource instanceof WebAssembly.Module;
      window.moduleSourcePhaseStaticIsAbstractModuleSource =
        staticSource instanceof AbstractModuleSource;
      window.moduleSourcePhaseStaticExports =
        WebAssembly.Module.exports(staticSource).map(({ name }) => name).sort().join(",");
      window.moduleSourcePhaseSharedIsModule =
        sharedSource instanceof WebAssembly.Module;
      sharedLogExec();
      window.moduleSourcePhaseSharedEvaluationResult =
        window.moduleStaticWasmImportLog.join(",");

      try {
        const dynamicSource = await import.source("/assets/wasm-import-from-wasm.wasm");
        window.moduleSourcePhaseDynamicIsModule =
          dynamicSource instanceof WebAssembly.Module;
        const instance = await WebAssembly.instantiate(dynamicSource, {
          "./wasm-export-to-wasm.wasm": {
            log() {
              window.moduleSourcePhaseDynamicLogged = true;
            }
          }
        });
        instance.exports.logExec();
        window.moduleSourcePhaseDynamicDone = true;
      } catch (error) {
        window.moduleSourcePhaseDynamicError =
          error && error.constructor
            ? error.constructor.name + ":" + error.message
            : String(error);
      }
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_source_phase_identity_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script type="module">
      try {
        const result = await import("/assets/module-source-phase-identity.js");
        window.moduleSourcePhaseIdentityNamespaceShared = result.namespaceShared;
        window.moduleSourcePhaseIdentitySourceShared = result.sourceShared;
        window.moduleSourcePhaseIdentityDone = true;
      } catch (error) {
        window.moduleSourcePhaseIdentityError =
          error && error.constructor
            ? error.constructor.name + ":" + error.message
            : String(error);
      }
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_source_phase_wasm_modulepreload_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script type="module">
      const wasmUrl = "/assets/module-wasm-exported-names.wasm";
      const absoluteWasmUrl = new URL(wasmUrl, location.href).href;
      const downloadCount = () =>
        performance
          .getEntriesByName(absoluteWasmUrl)
          .filter(entry => entry.transferSize > 0)
          .length;

      window.moduleSourcePhasePreloadDone = false;
      window.moduleSourcePhasePreloadError = "";
      const link = document.createElement("link");
      link.rel = "modulepreload";
      link.href = wasmUrl;
      link.onload = async () => {
        try {
          const entriesAfterPreload =
            performance.getEntriesByName(absoluteWasmUrl);
          window.moduleSourcePhasePreloadCountAfterLoad = downloadCount();
          window.moduleSourcePhasePreloadTransferPositive =
            entriesAfterPreload[0] && entriesAfterPreload[0].transferSize > 0;
          window.moduleSourcePhasePreloadInitiator =
            entriesAfterPreload[0] && entriesAfterPreload[0].initiatorType;
          const moduleExecuted = new Promise(resolve => {
            window.moduleSourcePhasePreloadResolve = resolve;
          });
          const script = document.createElement("script");
          script.type = "module";
          script.text = `
            const source = await import.source("${wasmUrl}");
            window.moduleSourcePhasePreloadImportIsModule =
              source instanceof WebAssembly.Module;
            window.moduleSourcePhasePreloadResolve();
          `;
          document.body.appendChild(script);
          await moduleExecuted;
          window.moduleSourcePhasePreloadCountAfterImport = downloadCount();
          window.moduleSourcePhasePreloadDone = true;
        } catch (error) {
          window.moduleSourcePhasePreloadError =
            error && error.constructor
              ? error.constructor.name + ":" + error.message
              : String(error);
        }
      };
      link.onerror = () => {
        window.moduleSourcePhasePreloadError = "link-error";
      };
      document.head.appendChild(link);
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_source_phase_wasm_dynamic_script_modulepreload_page()
-> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script>
      const wasmUrl = "../assets/execute-start.wasm";
      const absoluteWasmUrl = new URL(wasmUrl, location.href).href;
      const downloadCount = () =>
        performance
          .getEntriesByName(absoluteWasmUrl)
          .filter(entry => entry.transferSize > 0)
          .length;

      window.moduleSourcePhaseDynamicPreloadDone = false;
      window.moduleSourcePhaseDynamicPreloadError = "";
      window.moduleSourcePhaseDynamicScriptError = "";
      window.moduleSourcePhaseDynamicBeforeAwait = false;
      window.moduleSourcePhaseDynamicAfterAwait = false;
      window.moduleSourcePhaseDynamicScriptLoad = false;
      window.moduleSourcePhaseStaticDone = false;

      const staticLink = document.createElement("link");
      staticLink.rel = "modulepreload";
      staticLink.href = "../assets/module-wasm-exported-names.wasm";
      staticLink.onload = () => {
        const script = document.createElement("script");
        script.type = "module";
        script.text = `
          import source exportedNamesSource
            from "../assets/module-wasm-exported-names.wasm";
          window.moduleSourcePhaseStaticDone =
            exportedNamesSource instanceof WebAssembly.Module;
        `;
        document.body.appendChild(script);
      };
      staticLink.onerror = () => {
        window.moduleSourcePhaseDynamicPreloadError = "static-link-error";
      };
      document.head.appendChild(staticLink);

      const link = document.createElement("link");
      link.rel = "modulepreload";
      link.href = wasmUrl;
      link.onload = () => {
        window.moduleSourcePhaseDynamicPreloadCountAfterLoad = downloadCount();
        const script = document.createElement("script");
        script.type = "module";
        script.onerror = () => {
          window.moduleSourcePhaseDynamicScriptError = "script-error";
        };
        script.text = `
          window.moduleSourcePhaseDynamicBeforeAwait = true;
          try {
            await import.source("${wasmUrl}");
            window.moduleSourcePhaseDynamicAfterAwait = true;
            window.moduleSourcePhaseDynamicPreloadCountAfterImport =
              performance
                .getEntriesByName("${absoluteWasmUrl}")
                .filter(entry => entry.transferSize > 0)
                .length;
            window.moduleSourcePhaseDynamicPreloadDone = true;
          } catch (error) {
            window.moduleSourcePhaseDynamicScriptError =
              error && error.constructor
                ? error.constructor.name + ":" + error.message
                : String(error);
            throw error;
          }
        `;
        document.body.appendChild(script);
      };
      link.onerror = () => {
        window.moduleSourcePhaseDynamicPreloadError = "link-error";
      };
      document.head.appendChild(link);
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_external_wasm_script_executes_start_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script>
      window.moduleStaticWasmImportLog = [];
      window.moduleExternalWasmScriptErrored = false;
      window.addEventListener("load", () => {
        window.moduleExternalWasmScriptLog =
          window.moduleStaticWasmImportLog.join(",");
      });
    </script>
    <script
      type="module"
      src="/assets/execute-start.wasm"
      onerror="window.moduleExternalWasmScriptErrored = true"></script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_wasm_csp_blocks_cross_origin_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <head>
    <meta http-equiv="Content-Security-Policy" content="script-src 'self' 'unsafe-inline'">
  </head>
  <body>
    <script>
      window.moduleStaticWasmImportLog = [];
      window.moduleWasmCspViolationLog = [];
      document.addEventListener("securitypolicyviolation", (event) => {
        window.moduleWasmCspViolationLog.push([
          event.violatedDirective,
          event.effectiveDirective,
          event.blockedURI.endsWith("/assets/execute-start.wasm"),
          event instanceof SecurityPolicyViolationEvent
        ].join("|"));
        window.moduleWasmCspViolationCount =
          window.moduleWasmCspViolationLog.length;
      });
      window.addEventListener("load", () => {
        window.moduleWasmCspExecuted =
          window.moduleStaticWasmImportLog.join(",");
        window.moduleWasmCspViolationText =
          window.moduleWasmCspViolationLog.join(",");
      });
    </script>
    <script type="module">
      const wasmUrl = new URL("/assets/execute-start.wasm", location.href);
      wasmUrl.hostname =
        location.hostname === "localhost" ? "127.0.0.1" : "localhost";
      const script = document.createElement("script");
      script.type = "module";
      script.src = wasmUrl.href;
      script.onload = () => { window.moduleWasmCspScriptLoad = true; };
      script.onerror = () => { window.moduleWasmCspScriptError = true; };
      document.body.append(script);
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn wasm_api_csp_blocks_eval_from_response_header_page() -> Response {
    Response::builder()
        .header(CONTENT_TYPE, "text/html")
        .header(
            "Content-Security-Policy",
            "default-src 'self' 'unsafe-inline'",
        )
        .body(Body::from(
            r#"<!doctype html>
<html>
  <body>
    <script>
      window.wasmCspHeaderEvents = [];
      self.addEventListener("securitypolicyviolation", (event) => {
        window.wasmCspHeaderEvents.push([
          event.violatedDirective,
          event.effectiveDirective,
          event.originalPolicy,
          event.blockedURI,
          event instanceof SecurityPolicyViolationEvent
        ].join("|"));
        window.wasmCspHeaderEventText =
          window.wasmCspHeaderEvents.join(",");
      });

      WebAssembly.instantiate(
        new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0])
      ).then(
        () => { window.wasmCspHeaderResult = "resolved"; },
        (error) => {
          window.wasmCspHeaderResult = [
            error && error.constructor && error.constructor.name,
            error instanceof WebAssembly.CompileError
          ].join("|");
        }
      );
    </script>
  </body>
</html>"#,
        ))
        .expect("wasm CSP header page response should build")
}

pub(super) async fn wasm_module_postmessage_into_csp_iframe_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <iframe id="child" src="/compat/wasm-module-postmessage-csp-child"></iframe>
    <script>
      window.wasmPostMessageCspResult = "";
      window.wasmPostMessageCspUnexpectedMessage = false;
      window.addEventListener("message", (event) => {
        if (!event.data) {
          return;
        }
        if (event.data.kind === "unexpected-message") {
          window.wasmPostMessageCspUnexpectedMessage = true;
        } else if (event.data.kind === "wasm-csp") {
          window.wasmPostMessageCspResult = event.data.text;
        }
      });
      const child = document.getElementById("child");
      child.addEventListener("load", () => {
        const module = new WebAssembly.Module(
          new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0])
        );
        child.contentWindow.postMessage(module, "*");
      });
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn wasm_module_postmessage_csp_child_page() -> Response {
    Response::builder()
        .header(CONTENT_TYPE, "text/html")
        .header("Content-Security-Policy", "default-src 'unsafe-inline'")
        .body(Body::from(
            r#"<!doctype html>
<html>
  <body>
    <script>
      self.addEventListener("message", () => {
        parent.postMessage({ kind: "unexpected-message" }, "*");
      });
      self.addEventListener("securitypolicyviolation", (event) => {
        parent.postMessage({
          kind: "wasm-csp",
          text: [
            event.violatedDirective,
            event.effectiveDirective,
            event.originalPolicy,
            event.blockedURI,
            event instanceof SecurityPolicyViolationEvent
          ].join("|")
        }, "*");
      });
    </script>
  </body>
</html>"#,
        ))
        .expect("wasm postMessage CSP child response should build")
}

pub(super) async fn module_wasm_link_error_reports_typed_window_error_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script>
      window.moduleWasmLinkErrorConstructor = "";
      window.moduleWasmLinkErrorMessage = "";
      window.moduleWasmLinkErrorScriptLoad = false;
      window.moduleWasmLinkErrorScriptError = false;
      window.addEventListener("error", ev => {
        window.moduleWasmLinkErrorConstructor = ev.error.constructor.name;
        window.moduleWasmLinkErrorMessage = ev.message;
      });
    </script>
    <script
      type="module"
      src="/assets/js-wasm-cycle-function-error.js"
      onload="window.moduleWasmLinkErrorScriptLoad = true"
      onerror="window.moduleWasmLinkErrorScriptError = true"></script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_wasm_js_cycle_reports_guard_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script>
      window.moduleWasmJsCycleConstructor = "";
      window.moduleWasmJsCycleMessage = "";
      window.moduleWasmJsCycleUnexpected = false;
      window.addEventListener("error", ev => {
        window.moduleWasmJsCycleConstructor =
          ev.error && ev.error.constructor ? ev.error.constructor.name : "";
        window.moduleWasmJsCycleMessage = ev.message;
      });
    </script>
    <script type="module" src="/assets/document-wasm-js-cycle-entry.js"></script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_wasm_js_cycle_future_acceptance_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script type="module">
      window.moduleWasmJsCycleFutureDone = false;
      window.moduleWasmJsCycleFutureError = "";
      try {
        const wasm = await import("/assets/wasm-js-cycle.wasm");
        const js = await import("/assets/wasm-js-cycle.js");
        js.mutateBindings();

        const wasmGlobal = wasm.wasmGlob;
        window.moduleWasmJsCycleFutureGlobalType = typeof wasmGlobal;
        window.moduleWasmJsCycleFutureGlobalIsGlobal =
          wasmGlobal instanceof WebAssembly.Global;
        window.moduleWasmJsCycleFutureGlobalValue =
          wasmGlobal instanceof WebAssembly.Global
            ? wasmGlobal.valueOf()
            : wasmGlobal;

        window.moduleWasmJsCycleFutureFunction = wasm.wasmFunc();
        window.moduleWasmJsCycleFutureIncrementGlobal = wasm.incrementGlob();

        const memory = new Int32Array(wasm.wasmMem.buffer);
        window.moduleWasmJsCycleFutureMemoryBefore = memory[0];
        window.moduleWasmJsCycleFutureMutateMemory = wasm.mutateMem();
        window.moduleWasmJsCycleFutureMemoryAfter = memory[0];

        window.moduleWasmJsCycleFutureTableBeforeNull =
          wasm.wasmTab.get(0) === null;
        const tableRef = wasm.mutateTab();
        window.moduleWasmJsCycleFutureTableRefIsFunction =
          tableRef instanceof Function;
        window.moduleWasmJsCycleFutureTableAfterSame =
          wasm.wasmTab.get(0) === tableRef;
      } catch (error) {
        window.moduleWasmJsCycleFutureError =
          String(error && (error.stack || error.message || error));
      }
      window.moduleWasmJsCycleFutureDone = true;
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_js_wasm_cycle_function_import_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script>
      window.moduleJsWasmCycleConstructor = "";
      window.moduleJsWasmCycleMessage = "";
      window.moduleJsWasmCycleInitialRun = null;
      window.moduleJsWasmCycleAfterReassignRun = null;
      window.moduleJsWasmCycleImportedBinding = null;
      window.addEventListener("error", ev => {
        window.moduleJsWasmCycleConstructor =
          ev.error && ev.error.constructor ? ev.error.constructor.name : "";
        window.moduleJsWasmCycleMessage = ev.message;
      });
    </script>
    <script type="module">
      import { f } from "/assets/jscyc.js";
      window.moduleJsWasmCycleImportedBinding = f();
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_reserved_wasm_names_reject_with_link_error_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
  <body>
    <script type="module">
      const results = [];
      const unhandled = [];
      window.addEventListener("unhandledrejection", event => {
        unhandled.push(
          event.reason && event.reason.message
            ? event.reason.message
            : String(event.reason)
        );
      });
      const cases = [
        ["import-name", "/assets/invalid-wasm-import-name.wasm"],
        ["export-name", "/assets/invalid-wasm-export-name.wasm"],
        ["import-module", "/assets/invalid-wasm-import-module.wasm"],
      ];
      for (const [label, url] of cases) {
        try {
          await import(url);
          results.push(`${label}:resolved:false`);
        } catch (error) {
          const name = error && error.constructor && error.constructor.name;
          const isLinkError = error instanceof WebAssembly.LinkError;
          results.push(`${label}:${name}:${isLinkError}`);
        }
      }
      const OriginalLinkError = WebAssembly.LinkError;
      WebAssembly.LinkError = class PatchedLinkError extends Error {};
      try {
        await import("/assets/invalid-wasm-import-name.wasm?patched-link-error");
        results.push("patched-link-error:resolved:false");
      } catch (error) {
        const name = error && error.constructor && error.constructor.name;
        results.push([
          "patched-link-error",
          name,
          error.constructor === OriginalLinkError,
          error instanceof OriginalLinkError,
          error instanceof WebAssembly.LinkError,
        ].join(":"));
      }
      await new Promise(resolve => setTimeout(resolve, 0));
      window.moduleReservedWasmNameResults = results.join("|");
      window.moduleReservedWasmNameUnhandled = unhandled.join("|");
      window.moduleReservedWasmNameDone = true;
    </script>
  </body>
</html>"#,
    )
}

pub(super) async fn module_import_meta_resolve_page() -> Html<&'static str> {
    Html(MODULE_IMPORT_META_RESOLVE_HTML)
}

pub(super) async fn module_import_meta_resolve_comments_and_trailing_comma_page()
-> Html<&'static str> {
    Html(MODULE_IMPORT_META_RESOLVE_COMMENTS_AND_TRAILING_COMMA_HTML)
}

pub(super) async fn module_dynamic_import_template_literal_page() -> Html<&'static str> {
    Html(MODULE_DYNAMIC_IMPORT_TEMPLATE_LITERAL_HTML)
}

pub(super) async fn module_dynamic_import_string_compilation_base_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<meta charset="utf-8">
<base href="scripts/foo/">
<script>
  window.moduleDynamicImportStringCompilationOrder = ["script"];
  document.querySelector("base").remove();
  const base = document.createElement("base");
  base.setAttribute("href", "../");
  document.head.appendChild(base);

  Promise.resolve('import("../../../dynamic-import-target.mjs?eval").then(m => { window.moduleDynamicImportStringCompilationEval = m.label + ":" + m.urlSuffix; window.moduleDynamicImportStringCompilationOrder.push("eval"); })')
    .then(eval)
    .then(() => Function('return import("../../../dynamic-import-target.mjs?function").then(m => { window.moduleDynamicImportStringCompilationFunction = m.label + ":" + m.urlSuffix; window.moduleDynamicImportStringCompilationOrder.push("function"); })')())
    .then(() => {
      window.moduleDynamicImportStringCompilationDone = true;
      window.moduleDynamicImportStringCompilationFinalOrder = window.moduleDynamicImportStringCompilationOrder.join(",");
    }, error => {
      window.moduleDynamicImportStringCompilationError = String(error && (error.message || error));
      window.moduleDynamicImportStringCompilationDone = true;
      window.moduleDynamicImportStringCompilationFinalOrder = window.moduleDynamicImportStringCompilationOrder.join(",");
    });
</script>"#,
    )
}

pub(super) async fn module_escaped_string_literal_specifiers_page() -> Html<&'static str> {
    Html(MODULE_ESCAPED_STRING_LITERAL_SPECIFIERS_HTML)
}

pub(super) async fn module_export_variable_live_bindings_page() -> Html<&'static str> {
    Html(MODULE_EXPORT_VARIABLE_LIVE_BINDINGS_HTML)
}

pub(super) async fn module_self_bare_dynamic_import_resolves_after_own_evaluation_page()
-> Html<&'static str> {
    Html(MODULE_SELF_BARE_DYNAMIC_IMPORT_RESOLVES_AFTER_OWN_EVALUATION_HTML)
}

pub(super) async fn module_self_bare_dynamic_import_after_settle_resolves_page()
-> Html<&'static str> {
    Html(MODULE_SELF_BARE_DYNAMIC_IMPORT_AFTER_SETTLE_RESOLVES_HTML)
}

pub(super) async fn module_runtime_helper_shadowing_page() -> Html<&'static str> {
    Html(MODULE_RUNTIME_HELPER_SHADOWING_HTML)
}

pub(super) async fn module_multiline_import_and_export_list_page() -> Html<&'static str> {
    Html(MODULE_MULTILINE_IMPORT_AND_EXPORT_LIST_HTML)
}

pub(super) async fn module_export_star_and_namespace_reexport_page() -> Html<&'static str> {
    Html(MODULE_EXPORT_STAR_AND_NAMESPACE_REEXPORT_HTML)
}

pub(super) async fn module_cycle_dynamic_import_waits_for_target_evaluation_page()
-> Html<&'static str> {
    Html(MODULE_CYCLE_DYNAMIC_IMPORT_WAITS_FOR_TARGET_EVALUATION_HTML)
}

pub(super) async fn module_cycle_export_star_late_binding_page() -> Html<&'static str> {
    Html(MODULE_CYCLE_EXPORT_STAR_LATE_BINDING_HTML)
}

pub(super) async fn module_cycle_export_star_multihop_late_binding_page() -> Html<&'static str> {
    Html(MODULE_CYCLE_EXPORT_STAR_MULTIHOP_LATE_BINDING_HTML)
}

pub(super) async fn module_cycle_export_star_late_ambiguous_before_later_module_page()
-> Html<&'static str> {
    Html(MODULE_CYCLE_EXPORT_STAR_LATE_AMBIGUOUS_BEFORE_LATER_MODULE_HTML)
}

pub(super) async fn module_cycle_export_star_late_ambiguous_namespace_omits_export_page()
-> Html<&'static str> {
    Html(MODULE_CYCLE_EXPORT_STAR_LATE_AMBIGUOUS_NAMESPACE_OMITS_EXPORT_HTML)
}

pub(super) async fn module_cycle_export_star_multihop_late_ambiguous_namespace_omits_export_page()
-> Html<&'static str> {
    Html(MODULE_CYCLE_EXPORT_STAR_MULTIHOP_LATE_AMBIGUOUS_NAMESPACE_OMITS_EXPORT_HTML)
}

pub(super) async fn module_cycle_export_star_multihop_late_ambiguous_before_later_module_page()
-> Html<&'static str> {
    Html(MODULE_CYCLE_EXPORT_STAR_MULTIHOP_LATE_AMBIGUOUS_BEFORE_LATER_MODULE_HTML)
}

pub(super) async fn module_static_import_waits_for_initializing_non_cycle_dependency_page()
-> Html<&'static str> {
    Html(MODULE_STATIC_IMPORT_WAITS_FOR_INITIALIZING_NON_CYCLE_DEPENDENCY_HTML)
}

pub(super) async fn module_export_star_ambiguous_before_later_module_page() -> Html<&'static str> {
    Html(MODULE_EXPORT_STAR_AMBIGUOUS_BEFORE_LATER_MODULE_HTML)
}

pub(super) async fn module_cycle_missing_export_before_later_module_page() -> Html<&'static str> {
    Html(MODULE_CYCLE_MISSING_EXPORT_BEFORE_LATER_MODULE_HTML)
}

pub(super) async fn module_cycle_initializing_missing_export_before_later_module_page()
-> Html<&'static str> {
    Html(MODULE_CYCLE_INITIALIZING_MISSING_EXPORT_BEFORE_LATER_MODULE_HTML)
}

pub(super) async fn module_cycle_default_missing_from_export_star_before_later_module_page()
-> Html<&'static str> {
    Html(MODULE_CYCLE_DEFAULT_MISSING_FROM_EXPORT_STAR_BEFORE_LATER_MODULE_HTML)
}

pub(super) async fn parser_owned_module_missing_export_reports_window_error_after_restore_inline_page()
-> Html<&'static str> {
    Html(PARSER_OWNED_MODULE_MISSING_EXPORT_REPORTS_WINDOW_ERROR_AFTER_RESTORE_INLINE_HTML)
}

pub(super) async fn parser_owned_module_tla_rejection_reports_window_error_after_restore_inline_page()
-> Html<&'static str> {
    Html(PARSER_OWNED_MODULE_TLA_REJECTION_REPORTS_WINDOW_ERROR_AFTER_RESTORE_INLINE_HTML)
}

pub(super) async fn document_write_module_missing_export_reports_window_error_after_restore_inline_page()
-> Html<&'static str> {
    Html(DOCUMENT_WRITE_MODULE_MISSING_EXPORT_REPORTS_WINDOW_ERROR_AFTER_RESTORE_INLINE_HTML)
}

pub(super) async fn document_write_module_tla_rejection_reports_window_error_after_restore_inline_page()
-> Html<&'static str> {
    Html(DOCUMENT_WRITE_MODULE_TLA_REJECTION_REPORTS_WINDOW_ERROR_AFTER_RESTORE_INLINE_HTML)
}

pub(super) async fn parser_owned_module_pending_star_missing_export_before_later_module_page()
-> Html<&'static str> {
    Html(PARSER_OWNED_MODULE_PENDING_STAR_MISSING_EXPORT_BEFORE_LATER_MODULE_HTML)
}

pub(super) async fn parser_owned_module_pending_star_link_failure_before_body_and_later_module_page()
-> Html<&'static str> {
    Html(PARSER_OWNED_MODULE_PENDING_STAR_LINK_FAILURE_BEFORE_BODY_AND_LATER_MODULE_HTML)
}

pub(super) async fn parser_owned_module_pending_star_final_missing_reports_link_failure_page()
-> Html<&'static str> {
    Html(PARSER_OWNED_MODULE_PENDING_STAR_FINAL_MISSING_REPORTS_LINK_FAILURE_HTML)
}

pub(super) async fn module_shared_failed_dependency_is_not_reexecuted_page() -> Html<&'static str> {
    Html(MODULE_SHARED_FAILED_DEPENDENCY_IS_NOT_REEXECUTED_HTML)
}

pub(super) async fn stylesheet_media_change_load_handler_does_not_requeue_page()
-> Html<&'static str> {
    Html(STYLESHEET_MEDIA_CHANGE_LOAD_HANDLER_DOES_NOT_REQUEUE_HTML)
}

pub(super) async fn module_shared_unsupported_dependency_is_not_retried_page() -> Html<&'static str>
{
    Html(MODULE_SHARED_UNSUPPORTED_DEPENDENCY_IS_NOT_RETRIED_HTML)
}

pub(super) async fn module_top_level_await_delays_domcontentloaded_page() -> Html<&'static str> {
    Html(MODULE_TOP_LEVEL_AWAIT_DELAYS_DOMCONTENTLOADED_HTML)
}

pub(super) async fn module_top_level_await_over_fifty_ms_delays_domcontentloaded_page()
-> Html<&'static str> {
    Html(MODULE_TOP_LEVEL_AWAIT_OVER_FIFTY_MS_DELAYS_DOMCONTENTLOADED_HTML)
}

pub(super) async fn module_tla_dependency_delays_parent_and_domcontentloaded_page()
-> Html<&'static str> {
    Html(MODULE_TLA_DEPENDENCY_DELAYS_PARENT_AND_DOMCONTENTLOADED_HTML)
}

pub(super) async fn parser_owned_module_tla_dynamic_import_delays_domcontentloaded_page()
-> Html<&'static str> {
    Html(PARSER_OWNED_MODULE_TLA_DYNAMIC_IMPORT_DELAYS_DOMCONTENTLOADED_HTML)
}

pub(super) async fn parser_owned_module_error_before_later_module_page() -> Html<&'static str> {
    Html(PARSER_OWNED_MODULE_ERROR_BEFORE_LATER_MODULE_HTML)
}

pub(super) async fn parser_owned_module_missing_export_before_later_module_page()
-> Html<&'static str> {
    Html(PARSER_OWNED_MODULE_MISSING_EXPORT_BEFORE_LATER_MODULE_HTML)
}

pub(super) async fn parser_owned_module_missing_export_reports_window_error_before_later_module_page()
-> Html<&'static str> {
    Html(PARSER_OWNED_MODULE_MISSING_EXPORT_REPORTS_WINDOW_ERROR_BEFORE_LATER_MODULE_HTML)
}

pub(super) async fn parser_owned_module_missing_export_reports_window_error_payload_before_later_module_page()
-> Html<&'static str> {
    Html(PARSER_OWNED_MODULE_MISSING_EXPORT_REPORTS_WINDOW_ERROR_PAYLOAD_BEFORE_LATER_MODULE_HTML)
}

pub(super) async fn parser_owned_module_tla_rejection_before_later_module_page()
-> Html<&'static str> {
    Html(PARSER_OWNED_MODULE_TLA_REJECTION_BEFORE_LATER_MODULE_HTML)
}

pub(super) async fn parser_owned_module_tla_exotic_rejection_reports_window_error_payload_before_later_module_page()
-> Html<&'static str> {
    Html(PARSER_OWNED_MODULE_TLA_EXOTIC_REJECTION_REPORTS_WINDOW_ERROR_PAYLOAD_BEFORE_LATER_MODULE_HTML)
}

pub(super) async fn parser_owned_importmap_error_before_later_module_page() -> Html<&'static str> {
    Html(PARSER_OWNED_IMPORTMAP_ERROR_BEFORE_LATER_MODULE_HTML)
}

pub(super) async fn parser_owned_importmap_error_after_parser_progress_page() -> Html<&'static str>
{
    Html(PARSER_OWNED_IMPORTMAP_ERROR_AFTER_PARSER_PROGRESS_HTML)
}

pub(super) async fn dynamic_script_nomodule_commits_skip_page() -> Html<&'static str> {
    Html(DYNAMIC_SCRIPT_NOMODULE_COMMITS_SKIP_HTML)
}

pub(super) async fn dynamic_module_missing_default_export_does_not_abort_queue_page()
-> Html<&'static str> {
    Html(DYNAMIC_MODULE_MISSING_DEFAULT_EXPORT_DOES_NOT_ABORT_QUEUE_HTML)
}

pub(super) async fn dynamic_module_missing_default_export_reports_window_error_does_not_abort_queue_page()
-> Html<&'static str> {
    Html(DYNAMIC_MODULE_MISSING_DEFAULT_EXPORT_REPORTS_WINDOW_ERROR_DOES_NOT_ABORT_QUEUE_HTML)
}

pub(super) async fn dynamic_module_missing_default_export_reports_window_error_payload_does_not_abort_queue_page()
-> Html<&'static str> {
    Html(DYNAMIC_MODULE_MISSING_DEFAULT_EXPORT_REPORTS_WINDOW_ERROR_PAYLOAD_DOES_NOT_ABORT_QUEUE_HTML)
}

pub(super) async fn dynamic_module_tla_exotic_rejection_reports_window_error_payload_does_not_abort_queue_page()
-> Html<&'static str> {
    Html(DYNAMIC_MODULE_TLA_EXOTIC_REJECTION_REPORTS_WINDOW_ERROR_PAYLOAD_DOES_NOT_ABORT_QUEUE_HTML)
}

pub(super) async fn parse_time_defer_module_order_page() -> Html<&'static str> {
    Html(PARSE_TIME_DEFER_MODULE_ORDER_HTML)
}

pub(super) async fn parse_time_final_classic_terminal_before_dcl_page() -> Html<&'static str> {
    Html(PARSE_TIME_FINAL_CLASSIC_TERMINAL_BEFORE_DCL_HTML)
}

pub(super) async fn parse_time_final_module_terminal_before_dcl_page() -> Html<&'static str> {
    Html(PARSE_TIME_FINAL_MODULE_TERMINAL_BEFORE_DCL_HTML)
}

pub(super) async fn parse_time_lifecycle_tasks_page() -> Html<&'static str> {
    Html(PARSE_TIME_LIFECYCLE_TASKS_HTML)
}

pub(super) async fn abort_signal_any_page() -> Html<&'static str> {
    Html(ABORT_SIGNAL_ANY_HTML)
}

pub(super) async fn blob_urls_page() -> Html<&'static str> {
    Html(BLOB_URLS_HTML)
}

pub(super) async fn dom_rect_page() -> Html<&'static str> {
    Html(DOM_RECT_HTML)
}

pub(super) async fn image_data_page() -> Html<&'static str> {
    Html(IMAGE_DATA_HTML)
}

pub(super) async fn parser_image_fetch_policy_page(
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let token = params.get("token").map(String::as_str).unwrap_or("default");
    Html(PARSER_IMAGE_FETCH_POLICY_HTML.replace("{token}", token))
}

pub(super) async fn detached_eager_images_delay_load_page() -> Html<&'static str> {
    Html(DETACHED_EAGER_IMAGES_DELAY_LOAD_HTML)
}

pub(super) async fn asset_parser_image_fetch_policy_svg(
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Some(token) = params.get("token") {
        record_parser_image_fetch_policy_asset_request(token);
    }
    Response::builder()
        .header(CONTENT_TYPE, HeaderValue::from_static("image/svg+xml"))
        .body(Body::from(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"></svg>"#,
        ))
        .expect("valid parser image policy response")
}

pub(super) async fn asset_parser_image_fetch_policy_css(
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let token = params.get("token").map(String::as_str).unwrap_or("default");
    Response::builder()
        .header(CONTENT_TYPE, HeaderValue::from_static("text/css"))
        .body(Body::from(format!(
            "#css-image {{ background-image: url('/assets/parser-image-fetch-policy.svg?token={token}&source=css'); }}"
        )))
        .expect("valid parser image policy CSS response")
}

pub(super) async fn lazy_geometry_offset_chain_page() -> Html<&'static str> {
    Html(LAZY_GEOMETRY_OFFSET_CHAIN_HTML)
}

pub(super) async fn asset_detached_eager_image_slow_svg() -> Response {
    sleep(Duration::from_millis(150)).await;
    Response::builder()
        .header(CONTENT_TYPE, HeaderValue::from_static("image/svg+xml"))
        .body(Body::from(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"></svg>"#,
        ))
        .expect("valid slow detached eager image response")
}

pub(super) async fn web_streams_page() -> Html<&'static str> {
    Html(WEB_STREAMS_HTML)
}

pub(super) async fn intersection_observer_options_page() -> Html<&'static str> {
    Html(INTERSECTION_OBSERVER_OPTIONS_HTML)
}

pub(super) async fn intersection_observer_root_scope_page() -> Html<&'static str> {
    Html(INTERSECTION_OBSERVER_ROOT_SCOPE_HTML)
}

pub(super) async fn intersection_observer_root_geometry_page() -> Html<&'static str> {
    Html(INTERSECTION_OBSERVER_ROOT_GEOMETRY_HTML)
}

pub(super) async fn intersection_observer_thresholds_page() -> Html<&'static str> {
    Html(INTERSECTION_OBSERVER_THRESHOLDS_HTML)
}

pub(super) async fn mutation_observer_options_page() -> Html<&'static str> {
    Html(MUTATION_OBSERVER_OPTIONS_HTML)
}

pub(super) async fn mutation_observer_ordering_page() -> Html<&'static str> {
    Html(MUTATION_OBSERVER_ORDERING_HTML)
}

pub(super) async fn performance_measure_observer_page() -> Html<&'static str> {
    Html(PERFORMANCE_MEASURE_OBSERVER_HTML)
}

pub(super) async fn message_channel_page() -> Html<&'static str> {
    Html(MESSAGE_CHANNEL_HTML)
}

pub(super) async fn shared_worker_iframe_performance_owner_page() -> Html<&'static str> {
    Html(SHARED_WORKER_IFRAME_PERFORMANCE_OWNER_HTML)
}

pub(super) async fn asset_shared_worker_iframe_performance_owner() -> Response {
    Response::builder()
        .header(CONTENT_TYPE, HeaderValue::from_static("text/javascript"))
        .body(Body::from(SHARED_WORKER_IFRAME_PERFORMANCE_OWNER_JS))
        .expect("valid shared worker performance fixture response")
}

pub(super) async fn audio_worklet_wasm_source_phase_page() -> Html<&'static str> {
    Html(AUDIO_WORKLET_WASM_SOURCE_PHASE_HTML)
}

pub(super) async fn module_wasm_v128_global_export_throws_tdz_page() -> Html<&'static str> {
    Html(MODULE_WASM_V128_GLOBAL_EXPORT_THROWS_TDZ_HTML)
}

pub(super) async fn range_basic_page() -> Html<&'static str> {
    Html(RANGE_BASIC_HTML)
}

pub(super) async fn range_internal_algorithms_ignore_page_tampered_methods_page()
-> Html<&'static str> {
    Html(RANGE_INTERNAL_ALGORITHMS_IGNORE_PAGE_TAMPERED_METHODS_HTML)
}

pub(super) async fn selection_basic_page() -> Html<&'static str> {
    Html(SELECTION_BASIC_HTML)
}

pub(super) async fn selection_contains_node_ignores_page_tampered_node_contains_page()
-> Html<&'static str> {
    Html(SELECTION_CONTAINS_NODE_IGNORES_PAGE_TAMPERED_NODE_CONTAINS_HTML)
}

pub(super) async fn selection_set_base_and_extent_ignores_page_tampered_compare_document_position_page()
-> Html<&'static str> {
    Html(SELECTION_SET_BASE_AND_EXTENT_IGNORES_PAGE_TAMPERED_COMPARE_DOCUMENT_POSITION_HTML)
}

pub(super) async fn selectionchange_ignores_page_tampered_document_dispatch_event_page()
-> Html<&'static str> {
    Html(SELECTIONCHANGE_IGNORES_PAGE_TAMPERED_DOCUMENT_DISPATCH_EVENT_HTML)
}

pub(super) async fn form_data_ignores_page_tampered_node_contains_page() -> Html<&'static str> {
    Html(FORM_DATA_IGNORES_PAGE_TAMPERED_NODE_CONTAINS_HTML)
}

pub(super) async fn url_form_data_page() -> Html<&'static str> {
    Html(URL_FORM_DATA_HTML)
}

pub(super) async fn secondary_webapis_page() -> Html<&'static str> {
    Html(SECONDARY_WEBAPIS_HTML)
}

pub(super) async fn baidu_boot_compat_page() -> Html<&'static str> {
    Html(BAIDU_BOOT_COMPAT_HTML)
}

pub(super) async fn baidu_location_replace_boot_page() -> Html<&'static str> {
    Html(BAIDU_LOCATION_REPLACE_BOOT_HTML)
}

pub(super) async fn asset_script() -> Response {
    javascript_response(APP_JS)
}

pub(super) async fn asset_sequence_script() -> Response {
    javascript_response(SEQUENCE_JS)
}

pub(super) async fn asset_baidu_boot_compat_script() -> Response {
    javascript_response(BAIDU_BOOT_COMPAT_JS)
}

pub(super) async fn asset_baidu_location_replace_boot_script() -> Response {
    javascript_response(BAIDU_LOCATION_REPLACE_BOOT_JS)
}

pub(super) async fn asset_parse_time_classic_script() -> Response {
    javascript_response(PARSE_TIME_CLASSIC_JS)
}

pub(super) async fn asset_script_src_base_alpha_script() -> Response {
    javascript_response(SCRIPT_SRC_BASE_ALPHA_JS)
}

pub(super) async fn asset_script_src_base_beta_script() -> Response {
    javascript_response(SCRIPT_SRC_BASE_BETA_JS)
}

pub(super) async fn asset_parser_connected_load_write_te_script() -> Response {
    javascript_response(PARSER_CONNECTED_LOAD_WRITE_TE_JS)
}

pub(super) async fn asset_parse_time_defer_script() -> Response {
    javascript_response(PARSE_TIME_DEFER_JS)
}

pub(super) async fn asset_parse_time_async_script(headers: HeaderMap) -> Response {
    if let Some(host_key) = request_host_key(&headers) {
        notify_parse_time_async_chunked_tail_gate_if_present(&host_key);
    }
    javascript_response(PARSE_TIME_ASYNC_JS)
}

pub(super) async fn asset_parse_time_async_load_order_script() -> Response {
    javascript_response(PARSE_TIME_ASYNC_LOAD_ORDER_JS)
}

pub(super) async fn asset_runtime_owned_in_order_load_script() -> Response {
    javascript_response(RUNTIME_OWNED_IN_ORDER_LOAD_JS)
}

pub(super) async fn asset_missing_runtime_owned_in_order_error_script(
    headers: HeaderMap,
) -> Response {
    let host_key = request_host_key(&headers).unwrap_or_default();
    runtime_owned_in_order_error_after_dcl_gate(&host_key)
        .notified()
        .await;
    remove_runtime_owned_in_order_error_after_dcl_gate(&host_key);
    (StatusCode::NOT_FOUND, "Not Found").into_response()
}

pub(super) async fn asset_runtime_owned_in_order_load_slow_script() -> Response {
    sleep(Duration::from_millis(300)).await;
    javascript_response(RUNTIME_OWNED_IN_ORDER_LOAD_JS)
}

pub(super) async fn asset_runtime_owned_in_order_load_very_slow_script() -> Response {
    sleep(Duration::from_millis(1500)).await;
    javascript_response(RUNTIME_OWNED_IN_ORDER_LOAD_JS)
}

pub(super) async fn asset_runtime_owned_async_slow_script() -> Response {
    // `waitUntil: domcontentloaded` returns while the live page can continue to
    // load. Keep this fixture slow enough that DCL-cutoff tests observe the
    // DCL boundary instead of racing the later load event under nextest
    // concurrency.
    sleep(Duration::from_millis(300)).await;
    javascript_response(RUNTIME_OWNED_ASYNC_SLOW_JS)
}

pub(super) async fn asset_runtime_owned_async_fast_script(headers: HeaderMap) -> Response {
    if let Some(host_key) = request_host_key(&headers) {
        notify_runtime_owned_async_chunked_tail_gate_if_present(&host_key);
    }
    javascript_response(RUNTIME_OWNED_ASYNC_FAST_JS)
}

pub(super) async fn asset_runtime_owned_default_async_module_side_effect_module() -> Response {
    javascript_response(RUNTIME_OWNED_DEFAULT_ASYNC_MODULE_SIDE_EFFECT_MJS)
}

pub(super) async fn asset_parse_time_async_slow_script() -> Response {
    sleep(Duration::from_millis(75)).await;
    javascript_response(PARSE_TIME_ASYNC_SLOW_JS)
}

pub(super) async fn asset_parse_time_async_slow_chunked_first_script() -> Response {
    sleep(Duration::from_millis(75)).await;
    javascript_response(PARSE_TIME_ASYNC_SLOW_CHUNKED_FIRST_JS)
}

pub(super) async fn asset_parse_time_async_slow_chunked_defer_script() -> Response {
    javascript_response(PARSE_TIME_ASYNC_SLOW_CHUNKED_DEFER_JS)
}

pub(super) async fn asset_parse_time_async_task_first_script() -> Response {
    javascript_response(PARSE_TIME_ASYNC_TASK_FIRST_JS)
}

pub(super) async fn asset_parse_time_async_task_second_script() -> Response {
    sleep(Duration::from_millis(5)).await;
    javascript_response(PARSE_TIME_ASYNC_TASK_SECOND_JS)
}

pub(super) async fn asset_parse_time_async_task_visibility_first_script() -> Response {
    javascript_response(PARSE_TIME_ASYNC_TASK_VISIBILITY_FIRST_JS)
}

pub(super) async fn asset_parse_time_async_task_visibility_second_script() -> Response {
    sleep(Duration::from_millis(5)).await;
    javascript_response(PARSE_TIME_ASYNC_TASK_VISIBILITY_SECOND_JS)
}

pub(super) async fn asset_parse_time_async_post_parse_first_script() -> Response {
    // This fixture asserts the post-DCL fallback path, so keep the completion
    // comfortably beyond the tiny parse/DCL window on local test servers.
    sleep(Duration::from_millis(30)).await;
    javascript_response(PARSE_TIME_ASYNC_POST_PARSE_FIRST_JS)
}

pub(super) async fn asset_parse_time_async_post_parse_slow_first_script() -> Response {
    javascript_response(PARSE_TIME_ASYNC_POST_PARSE_SLOW_FIRST_JS)
}

pub(super) async fn asset_parse_time_async_post_parse_second_script() -> Response {
    // Keep both "post-parse" fixture assets outside the completion-before-
    // handoff window. The Chromium-aligned scheduler intentionally allows that
    // window to execute before DCL, so a zero-delay route here is inherently
    // flaky.
    sleep(Duration::from_millis(30)).await;
    javascript_response(PARSE_TIME_ASYNC_POST_PARSE_SECOND_JS)
}

pub(super) async fn asset_parse_time_async_post_parse_slow_second_script() -> Response {
    // Keep this comfortably beyond the post-parse compat window so the fixture
    // remains stable under full-suite parallel load. The test is asserting the
    // DCL fallback boundary, not a razor-thin race against scheduler latency.
    sleep(Duration::from_millis(80)).await;
    javascript_response(PARSE_TIME_ASYNC_POST_PARSE_SLOW_SECOND_JS)
}

pub(super) async fn asset_parse_time_defer_left_script() -> Response {
    javascript_response(PARSE_TIME_DEFER_LEFT_JS)
}

pub(super) async fn asset_parse_time_defer_right_script() -> Response {
    javascript_response(PARSE_TIME_DEFER_RIGHT_JS)
}

pub(super) async fn asset_parse_time_final_classic_terminal_script() -> Response {
    javascript_response(PARSE_TIME_FINAL_CLASSIC_TERMINAL_JS)
}

pub(super) async fn asset_parse_time_final_module_terminal_script() -> Response {
    javascript_response(PARSE_TIME_FINAL_MODULE_TERMINAL_JS)
}

pub(super) async fn asset_parse_time_lifecycle_defer_script() -> Response {
    javascript_response(PARSE_TIME_LIFECYCLE_DEFER_JS)
}

pub(super) async fn asset_parse_time_lifecycle_async_script() -> Response {
    sleep(Duration::from_millis(25)).await;
    javascript_response(PARSE_TIME_LIFECYCLE_ASYNC_JS)
}

pub(super) async fn asset_blocking_stylesheet_parser_blocking_script() -> Response {
    javascript_response(BLOCKING_STYLESHEET_PARSER_BLOCKING_JS)
}

pub(super) async fn asset_blocking_stylesheet_parser_blocking_document_write_script() -> Response {
    javascript_response(BLOCKING_STYLESHEET_PARSER_BLOCKING_DOCUMENT_WRITE_JS)
}

pub(super) async fn asset_blocking_stylesheet_defer_script() -> Response {
    javascript_response(BLOCKING_STYLESHEET_DEFER_JS)
}

pub(super) async fn asset_phase_two_upgrade_runtime_style_defer_script() -> Response {
    javascript_response(PHASE_TWO_UPGRADE_RUNTIME_STYLE_DEFER_JS)
}

pub(super) async fn asset_blocking_stylesheet_alternate_probe_script() -> Response {
    javascript_response(BLOCKING_STYLESHEET_ALTERNATE_PROBE_JS)
}

pub(super) async fn asset_dynamic_blocking_stylesheet_runtime_script(
    Extension(state): Extension<FixtureRuntimeState>,
) -> Response {
    state.dynamic_stylesheet_dcl.wait().await;
    javascript_response(DYNAMIC_BLOCKING_STYLESHEET_RUNTIME_JS)
}

pub(super) async fn asset_dynamic_blocking_stylesheet_dcl(
    Extension(state): Extension<FixtureRuntimeState>,
) -> StatusCode {
    state.dynamic_stylesheet_dcl.signal();
    StatusCode::NO_CONTENT
}

pub(super) async fn asset_dynamic_blocking_stylesheet_script_executed(
    Extension(state): Extension<FixtureRuntimeState>,
) -> StatusCode {
    state.dynamic_stylesheet_script_executed.signal();
    StatusCode::NO_CONTENT
}

pub(super) async fn asset_dynamic_taxonomy_async_fast_script() -> Response {
    javascript_response(DYNAMIC_TAXONOMY_ASYNC_FAST_JS)
}

pub(super) async fn asset_dynamic_taxonomy_in_order_slow_script() -> Response {
    sleep(Duration::from_millis(40)).await;
    javascript_response(DYNAMIC_TAXONOMY_IN_ORDER_SLOW_JS)
}

pub(super) async fn asset_dynamic_taxonomy_in_order_fast_script() -> Response {
    javascript_response(DYNAMIC_TAXONOMY_IN_ORDER_FAST_JS)
}

pub(super) async fn asset_parse_time_dynamic_clobber_script() -> Response {
    javascript_response(PARSE_TIME_DYNAMIC_CLOBBER_JS)
}

pub(super) async fn asset_parse_time_dynamic_followup_script() -> Response {
    javascript_response(PARSE_TIME_DYNAMIC_FOLLOWUP_JS)
}

pub(super) async fn asset_parse_time_dynamic_error_clobber_script() -> Response {
    javascript_response(PARSE_TIME_DYNAMIC_ERROR_CLOBBER_JS)
}

pub(super) async fn asset_document_write_page_task_clobber_script() -> Response {
    javascript_response(DOCUMENT_WRITE_PAGE_TASK_CLOBBER_JS)
}

pub(super) async fn asset_document_write_delayed_external_script() -> Response {
    sleep(Duration::from_millis(80)).await;
    javascript_response(DOCUMENT_WRITE_DELAYED_EXTERNAL_JS)
}

pub(super) async fn asset_document_open_parser_external_script() -> Response {
    sleep(Duration::from_millis(80)).await;
    javascript_response(DOCUMENT_OPEN_PARSER_EXTERNAL_JS)
}

pub(super) async fn asset_document_write_nested_external_parent_script() -> Response {
    javascript_response(DOCUMENT_WRITE_NESTED_EXTERNAL_PARENT_JS)
}

pub(super) async fn asset_document_write_nested_external_child_script() -> Response {
    sleep(Duration::from_millis(10)).await;
    javascript_response(DOCUMENT_WRITE_NESTED_EXTERNAL_CHILD_JS)
}

pub(super) async fn asset_document_write_nested_external_outer_after_script() -> Response {
    javascript_response(DOCUMENT_WRITE_NESTED_EXTERNAL_OUTER_AFTER_JS)
}

pub(super) async fn asset_document_write_external_split_session_parent_script() -> Response {
    sleep(Duration::from_millis(10)).await;
    javascript_response(DOCUMENT_WRITE_EXTERNAL_SPLIT_SESSION_PARENT_JS)
}

pub(super) async fn asset_document_write_inserted_chunked_external_script() -> Response {
    sleep(Duration::from_millis(50)).await;
    javascript_response(DOCUMENT_WRITE_INSERTED_CHUNKED_EXTERNAL_JS)
}

pub(super) async fn asset_dynamic_preparation_context_stale_script() -> Response {
    sleep(Duration::from_millis(40)).await;
    javascript_response(DYNAMIC_PREPARATION_CONTEXT_STALE_JS)
}

pub(super) async fn asset_dynamic_preparation_context_open_script() -> Response {
    javascript_response(DYNAMIC_PREPARATION_CONTEXT_OPEN_JS)
}

pub(super) async fn asset_document_write_implicit_replace_async_script() -> Response {
    sleep(Duration::from_millis(10)).await;
    javascript_response(DOCUMENT_WRITE_IMPLICIT_REPLACE_ASYNC_JS)
}

pub(super) async fn asset_document_write_implicit_replace_async_module_script() -> Response {
    sleep(Duration::from_millis(10)).await;
    javascript_response(DOCUMENT_WRITE_IMPLICIT_REPLACE_ASYNC_MODULE_JS)
}

pub(super) async fn asset_document_write_replacement_async_boot_script() -> Response {
    javascript_response(DOCUMENT_WRITE_REPLACEMENT_ASYNC_BOOT_JS)
}

pub(super) async fn asset_document_write_replacement_async_script() -> Response {
    // `async` scripts do not block DOMContentLoaded. Delay this fixture so the
    // test's "after DOMContentLoaded" assertion follows from source readiness
    // rather than from one implementation's scheduler speed.
    sleep(Duration::from_millis(10)).await;
    javascript_response(DOCUMENT_WRITE_REPLACEMENT_ASYNC_JS)
}

pub(super) async fn asset_document_write_implicit_replace_stale_defer_script() -> Response {
    javascript_response(DOCUMENT_WRITE_IMPLICIT_REPLACE_STALE_DEFER_JS)
}

pub(super) async fn asset_document_write_implicit_replace_stale_module_script() -> Response {
    javascript_response(DOCUMENT_WRITE_IMPLICIT_REPLACE_STALE_MODULE_MJS)
}

pub(super) async fn asset_document_write_external_parser_blocking_script() -> Response {
    sleep(Duration::from_millis(10)).await;
    javascript_response(DOCUMENT_WRITE_EXTERNAL_PARSER_BLOCKING_JS)
}

pub(super) async fn asset_document_write_load_microtask_script() -> Response {
    sleep(Duration::from_millis(10)).await;
    javascript_response(DOCUMENT_WRITE_LOAD_MICROTASK_JS)
}

pub(super) async fn asset_document_write_defer_written_script() -> Response {
    sleep(Duration::from_millis(10)).await;
    javascript_response(DOCUMENT_WRITE_DEFER_WRITTEN_JS)
}

pub(super) async fn asset_document_write_defer_written_dcl_script() -> Response {
    sleep(Duration::from_millis(10)).await;
    javascript_response(DOCUMENT_WRITE_DEFER_WRITTEN_DCL_JS)
}

pub(super) async fn asset_document_open_after_load_external_1_script() -> Response {
    javascript_response(DOCUMENT_OPEN_AFTER_LOAD_EXTERNAL_1_JS)
}

pub(super) async fn asset_document_open_after_load_external_2_script() -> Response {
    javascript_response(DOCUMENT_OPEN_AFTER_LOAD_EXTERNAL_2_JS)
}

pub(super) async fn asset_document_write_importmap_written_module_script() -> Response {
    javascript_response(DOCUMENT_WRITE_IMPORTMAP_WRITTEN_MODULE_MJS)
}

pub(super) async fn asset_importmap_scoped_imported_module() -> Response {
    javascript_response(IMPORTMAP_SCOPED_IMPORTED_MJS)
}

pub(super) async fn asset_importmap_initial_target_module() -> Response {
    javascript_response(IMPORTMAP_INITIAL_TARGET_MJS)
}

pub(super) async fn asset_importmap_override_target_module() -> Response {
    javascript_response(IMPORTMAP_OVERRIDE_TARGET_MJS)
}

pub(super) async fn asset_importmap_extra_target_module() -> Response {
    javascript_response(IMPORTMAP_EXTRA_TARGET_MJS)
}

pub(super) async fn asset_importmap_canonical_target_module() -> Response {
    javascript_response(IMPORTMAP_CANONICAL_TARGET_MJS)
}

pub(super) async fn asset_module_tla_dependency() -> Response {
    javascript_response(MODULE_TLA_DEPENDENCY_MJS)
}

pub(super) async fn asset_parser_owned_module_tla_dynamic_import_dep() -> Response {
    sleep(Duration::from_secs(1)).await;
    javascript_response(PARSER_OWNED_MODULE_TLA_DYNAMIC_IMPORT_DEP_MJS)
}

pub(super) async fn asset_stylesheet_media_change_load_handler_css() -> Response {
    css_response(STYLESHEET_MEDIA_CHANGE_LOAD_HANDLER_CSS)
}

pub(super) async fn asset_module_default_export_value() -> Response {
    javascript_response(MODULE_DEFAULT_EXPORT_VALUE_MJS)
}

pub(super) async fn module_dependency_fetch_uses_module_credentials_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html><html><body data-module-dependency-cookie="pending"><script type="module" src="/assets/module-dependency-cookie-root.mjs"></script></body></html>"#,
    )
}

pub(super) async fn asset_module_dependency_cookie_root() -> Response {
    javascript_response(
        r#"import { seen } from "/assets/module-dependency-cookie-leaf.mjs";
document.body.setAttribute("data-module-dependency-cookie", String(seen));"#,
    )
}

pub(super) async fn asset_module_dependency_cookie_leaf(headers: HeaderMap) -> Response {
    javascript_string_response(format!(
        "export const seen = {};",
        has_cookie(&headers, "session=fixture")
    ))
}

pub(super) async fn asset_module_default_reexport_source() -> Response {
    javascript_response(MODULE_DEFAULT_REEXPORT_SOURCE_MJS)
}

pub(super) async fn asset_module_default_reexport_barrel() -> Response {
    javascript_response(MODULE_DEFAULT_REEXPORT_BARREL_MJS)
}

pub(super) async fn asset_module_string_literal_export_names_source() -> Response {
    javascript_response(MODULE_STRING_LITERAL_EXPORT_NAMES_SOURCE_MJS)
}

pub(super) async fn asset_module_string_literal_export_names_barrel() -> Response {
    javascript_response(MODULE_STRING_LITERAL_EXPORT_NAMES_BARREL_MJS)
}

pub(super) async fn asset_module_string_literal_export_names_surrogate_pairs_source() -> Response {
    javascript_response(MODULE_STRING_LITERAL_EXPORT_NAMES_SURROGATE_PAIRS_SOURCE_MJS)
}

pub(super) async fn asset_module_string_literal_export_names_surrogate_pairs_barrel() -> Response {
    javascript_response(MODULE_STRING_LITERAL_EXPORT_NAMES_SURROGATE_PAIRS_BARREL_MJS)
}

pub(super) async fn asset_module_export_star_string_literal_namespace_barrel() -> Response {
    javascript_response(MODULE_EXPORT_STAR_STRING_LITERAL_NAMESPACE_BARREL_MJS)
}

pub(super) async fn asset_module_escaped_identifier_names() -> Response {
    javascript_response(MODULE_ESCAPED_IDENTIFIER_NAMES_MJS)
}

pub(super) async fn asset_module_export_destructuring_bindings() -> Response {
    javascript_response(MODULE_EXPORT_DESTRUCTURING_BINDINGS_MJS)
}

pub(super) async fn asset_module_export_nested_destructuring_bindings() -> Response {
    javascript_response(MODULE_EXPORT_NESTED_DESTRUCTURING_BINDINGS_MJS)
}

pub(super) async fn asset_module_export_nested_initializer_commas() -> Response {
    javascript_response(MODULE_EXPORT_NESTED_INITIALIZER_COMMAS_MJS)
}

pub(super) async fn asset_module_import_export_list_comments_source() -> Response {
    javascript_response(MODULE_IMPORT_EXPORT_LIST_COMMENTS_SOURCE_MJS)
}

pub(super) async fn asset_module_import_export_list_comments_barrel() -> Response {
    javascript_response(MODULE_IMPORT_EXPORT_LIST_COMMENTS_BARREL_MJS)
}

pub(super) async fn asset_module_dynamic_import_comments_target() -> Response {
    javascript_response(MODULE_DYNAMIC_IMPORT_COMMENTS_TARGET_MJS)
}

pub(super) async fn asset_dynamic_import_document_write_target() -> Response {
    javascript_response(
        r#"document.write("document.write body contents\n");
document.close();
parent.dynamicImportDocumentWriteBody = document.body.textContent;
parent.dynamicImportDocumentWriteDone = true;
"#,
    )
}

pub(super) async fn asset_module_text_import_target() -> Response {
    Response::builder()
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from("text file\n"))
        .expect("text module fixture response should build")
}

pub(super) async fn asset_module_synthetic_data_json() -> Response {
    Response::builder()
        .header(CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(r#"{"answer":42,"label":"json-ok"}"#))
        .expect("JSON module fixture response should build")
}

pub(super) async fn asset_module_synthetic_style_css() -> Response {
    css_response("body { color: rgb(1, 2, 3); }")
}

pub(super) async fn asset_module_wasm_exported_names() -> Response {
    const MODULE_WASM_EXPORTED_NAMES: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03,
        0x02, 0x01, 0x00, 0x04, 0x04, 0x01, 0x6f, 0x00, 0x0a, 0x05, 0x04, 0x01, 0x01, 0x00, 0x0a,
        0x06, 0x06, 0x01, 0x7f, 0x00, 0x41, 0x00, 0x0b, 0x07, 0x1b, 0x04, 0x04, 0x67, 0x6c, 0x6f,
        0x62, 0x03, 0x00, 0x03, 0x6d, 0x65, 0x6d, 0x02, 0x00, 0x03, 0x74, 0x61, 0x62, 0x01, 0x00,
        0x04, 0x66, 0x75, 0x6e, 0x63, 0x00, 0x00, 0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b,
    ];
    (
        [(CONTENT_TYPE, "application/wasm")],
        MODULE_WASM_EXPORTED_NAMES.to_vec(),
    )
        .into_response()
}

pub(super) async fn asset_wasm_import_from_wasm() -> Response {
    const WASM_IMPORT_FROM_WASM: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x02,
        0x22, 0x01, 0x1a, 0x2e, 0x2f, 0x77, 0x61, 0x73, 0x6d, 0x2d, 0x65, 0x78, 0x70, 0x6f, 0x72,
        0x74, 0x2d, 0x74, 0x6f, 0x2d, 0x77, 0x61, 0x73, 0x6d, 0x2e, 0x77, 0x61, 0x73, 0x6d, 0x03,
        0x6c, 0x6f, 0x67, 0x00, 0x00, 0x03, 0x02, 0x01, 0x00, 0x07, 0x0b, 0x01, 0x07, 0x6c, 0x6f,
        0x67, 0x45, 0x78, 0x65, 0x63, 0x00, 0x01, 0x0a, 0x06, 0x01, 0x04, 0x00, 0x10, 0x00, 0x0b,
    ];
    (
        [(CONTENT_TYPE, "application/wasm")],
        WASM_IMPORT_FROM_WASM.to_vec(),
    )
        .into_response()
}

pub(super) async fn asset_wasm_export_to_wasm() -> Response {
    const WASM_EXPORT_TO_WASM: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x02,
        0x14, 0x01, 0x08, 0x2e, 0x2f, 0x6c, 0x6f, 0x67, 0x2e, 0x6a, 0x73, 0x07, 0x6c, 0x6f, 0x67,
        0x45, 0x78, 0x65, 0x63, 0x00, 0x00, 0x07, 0x07, 0x01, 0x03, 0x6c, 0x6f, 0x67, 0x00, 0x00,
    ];
    (
        [(CONTENT_TYPE, "application/wasm")],
        WASM_EXPORT_TO_WASM.to_vec(),
    )
        .into_response()
}

pub(super) async fn asset_wasm_import_js_dependency() -> Response {
    const WASM_IMPORT_JS_DEPENDENCY: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x02,
        0x14, 0x01, 0x08, 0x2e, 0x2f, 0x64, 0x65, 0x70, 0x2e, 0x6a, 0x73, 0x07, 0x6c, 0x6f, 0x67,
        0x45, 0x78, 0x65, 0x63, 0x00, 0x00, 0x07, 0x07, 0x01, 0x03, 0x6c, 0x6f, 0x67, 0x00, 0x00,
    ];
    (
        [(CONTENT_TYPE, "application/wasm")],
        WASM_IMPORT_JS_DEPENDENCY.to_vec(),
    )
        .into_response()
}

pub(super) async fn asset_wasm_import_bad_js_dependency() -> Response {
    const WASM_IMPORT_BAD_JS_DEPENDENCY: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x02,
        0x14, 0x01, 0x08, 0x2e, 0x2f, 0x62, 0x61, 0x64, 0x2e, 0x6a, 0x73, 0x07, 0x6c, 0x6f, 0x67,
        0x45, 0x78, 0x65, 0x63, 0x00, 0x00, 0x07, 0x07, 0x01, 0x03, 0x6c, 0x6f, 0x67, 0x00, 0x00,
    ];
    (
        [(CONTENT_TYPE, "application/wasm")],
        WASM_IMPORT_BAD_JS_DEPENDENCY.to_vec(),
    )
        .into_response()
}

pub(super) async fn asset_wasm_log_module() -> Response {
    javascript_response(
        r#"export function logExec() {
  window.moduleStaticWasmImportLog.push("executed");
}"#,
    )
}

pub(super) async fn asset_wasm_log_dependency_module() -> Response {
    javascript_response(r#"export { logExec } from "./leaf.js";"#)
}

pub(super) async fn asset_wasm_throwing_dependency_module() -> Response {
    javascript_response(
        r#"throw new WebAssembly.LinkError("dependency-link-boom");
export function logExec() {}"#,
    )
}

pub(super) async fn asset_wasm_log_leaf_module() -> Response {
    javascript_response(
        r#"export function logExec() {
  window.moduleStaticWasmImportJsDependencyLog.push("leaf");
}"#,
    )
}

pub(super) async fn asset_wasm_import_throwing_js_dependency_entry_module() -> Response {
    javascript_response(
        r#"import { log } from "./wasm-import-bad-js-dependency.wasm";
log();
window.moduleStaticWasmThrowingJsDependencyLoaded = true;"#,
    )
}

pub(super) async fn asset_audio_worklet_source_phase_module() -> Response {
    javascript_response(
        r#"import source modSource from "/assets/module-wasm-exported-names.wasm";

class AudioProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.port.onmessage = async () => {
      let dynamicCheck = false;
      try {
        await import.source("/assets/execute-start.wasm");
      } catch (error) {
        dynamicCheck = error instanceof TypeError;
      }
      this.port.postMessage({
        value: 42,
        staticCheck: modSource instanceof WebAssembly.Module,
        dynamicCheck,
        exports: WebAssembly.Module.exports(modSource).map(({ name }) => name).sort().join(",")
      });
    };
  }

  process() {
    return true;
  }
}

registerProcessor("audio-processor", AudioProcessor);
"#,
    )
}

pub(super) async fn asset_mutable_global_wasm() -> Response {
    const MUTABLE_GLOBAL_WASM: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03,
        0x02, 0x01, 0x00, 0x04, 0x04, 0x01, 0x6f, 0x00, 0x0a, 0x05, 0x04, 0x01, 0x01, 0x00, 0x0a,
        0x06, 0x06, 0x01, 0x7f, 0x01, 0x41, 0x00, 0x0b, 0x07, 0x1b, 0x04, 0x04, 0x67, 0x6c, 0x6f,
        0x62, 0x03, 0x00, 0x03, 0x6d, 0x65, 0x6d, 0x02, 0x00, 0x03, 0x74, 0x61, 0x62, 0x01, 0x00,
        0x04, 0x66, 0x75, 0x6e, 0x63, 0x00, 0x00, 0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b,
    ];
    (
        [(CONTENT_TYPE, "application/wasm")],
        MUTABLE_GLOBAL_WASM.to_vec(),
    )
        .into_response()
}

pub(super) async fn asset_mutable_global_with_v128_wasm() -> Response {
    const MUTABLE_GLOBAL_WITH_V128_WASM: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x15, 0x04, 0x60, 0x01, 0x7f, 0x00,
        0x60, 0x00, 0x01, 0x7f, 0x60, 0x04, 0x7f, 0x7f, 0x7f, 0x7f, 0x00, 0x60, 0x01, 0x7f, 0x01,
        0x7f, 0x03, 0x05, 0x04, 0x00, 0x01, 0x02, 0x03, 0x06, 0x1c, 0x02, 0x7f, 0x01, 0x41, 0xe4,
        0x00, 0x0b, 0x7b, 0x01, 0xfd, 0x0c, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x03,
        0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x0b, 0x07, 0x53, 0x06, 0x0c, 0x6d, 0x75, 0x74,
        0x61, 0x62, 0x6c, 0x65, 0x56, 0x61, 0x6c, 0x75, 0x65, 0x03, 0x00, 0x0a, 0x76, 0x31, 0x32,
        0x38, 0x45, 0x78, 0x70, 0x6f, 0x72, 0x74, 0x03, 0x01, 0x09, 0x73, 0x65, 0x74, 0x47, 0x6c,
        0x6f, 0x62, 0x61, 0x6c, 0x00, 0x00, 0x09, 0x67, 0x65, 0x74, 0x47, 0x6c, 0x6f, 0x62, 0x61,
        0x6c, 0x00, 0x01, 0x0d, 0x73, 0x65, 0x74, 0x56, 0x31, 0x32, 0x38, 0x47, 0x6c, 0x6f, 0x62,
        0x61, 0x6c, 0x00, 0x02, 0x0b, 0x67, 0x65, 0x74, 0x56, 0x31, 0x32, 0x38, 0x4c, 0x61, 0x6e,
        0x65, 0x00, 0x03, 0x0a, 0x5c, 0x04, 0x06, 0x00, 0x20, 0x00, 0x24, 0x00, 0x0b, 0x04, 0x00,
        0x23, 0x00, 0x0b, 0x1c, 0x00, 0x41, 0x00, 0xfd, 0x11, 0x20, 0x00, 0xfd, 0x1c, 0x00, 0x20,
        0x01, 0xfd, 0x1c, 0x01, 0x20, 0x02, 0xfd, 0x1c, 0x02, 0x20, 0x03, 0xfd, 0x1c, 0x03, 0x24,
        0x01, 0x0b, 0x31, 0x00, 0x20, 0x00, 0x41, 0x00, 0x46, 0x04, 0x7f, 0x23, 0x01, 0xfd, 0x1b,
        0x00, 0x05, 0x20, 0x00, 0x41, 0x01, 0x46, 0x04, 0x7f, 0x23, 0x01, 0xfd, 0x1b, 0x01, 0x05,
        0x20, 0x00, 0x41, 0x02, 0x46, 0x04, 0x7f, 0x23, 0x01, 0xfd, 0x1b, 0x02, 0x05, 0x23, 0x01,
        0xfd, 0x1b, 0x03, 0x0b, 0x0b, 0x0b, 0x0b, 0x00, 0x80, 0x01, 0x04, 0x6e, 0x61, 0x6d, 0x65,
        0x01, 0x33, 0x04, 0x00, 0x09, 0x73, 0x65, 0x74, 0x47, 0x6c, 0x6f, 0x62, 0x61, 0x6c, 0x01,
        0x09, 0x67, 0x65, 0x74, 0x47, 0x6c, 0x6f, 0x62, 0x61, 0x6c, 0x02, 0x0d, 0x73, 0x65, 0x74,
        0x56, 0x31, 0x32, 0x38, 0x47, 0x6c, 0x6f, 0x62, 0x61, 0x6c, 0x03, 0x0b, 0x67, 0x65, 0x74,
        0x56, 0x31, 0x32, 0x38, 0x4c, 0x61, 0x6e, 0x65, 0x02, 0x24, 0x03, 0x00, 0x01, 0x00, 0x09,
        0x6e, 0x65, 0x77, 0x5f, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x02, 0x04, 0x00, 0x01, 0x78, 0x01,
        0x01, 0x79, 0x02, 0x01, 0x7a, 0x03, 0x01, 0x77, 0x03, 0x01, 0x00, 0x04, 0x6c, 0x61, 0x6e,
        0x65, 0x07, 0x1e, 0x02, 0x00, 0x0d, 0x6d, 0x75, 0x74, 0x61, 0x62, 0x6c, 0x65, 0x5f, 0x76,
        0x61, 0x6c, 0x75, 0x65, 0x01, 0x0c, 0x6d, 0x75, 0x74, 0x61, 0x62, 0x6c, 0x65, 0x5f, 0x76,
        0x31, 0x32, 0x38,
    ];
    (
        [(CONTENT_TYPE, "application/wasm")],
        MUTABLE_GLOBAL_WITH_V128_WASM.to_vec(),
    )
        .into_response()
}

pub(super) async fn asset_mutable_global_reexport_wasm() -> Response {
    const MUTABLE_GLOBAL_REEXPORT_WASM: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0e, 0x03, 0x60, 0x01, 0x7f, 0x00,
        0x60, 0x00, 0x01, 0x7f, 0x60, 0x01, 0x7f, 0x01, 0x7f, 0x02, 0x59, 0x02, 0x1c, 0x2e, 0x2f,
        0x6d, 0x75, 0x74, 0x61, 0x62, 0x6c, 0x65, 0x2d, 0x67, 0x6c, 0x6f, 0x62, 0x61, 0x6c, 0x2d,
        0x65, 0x78, 0x70, 0x6f, 0x72, 0x74, 0x2e, 0x77, 0x61, 0x73, 0x6d, 0x0c, 0x6d, 0x75, 0x74,
        0x61, 0x62, 0x6c, 0x65, 0x56, 0x61, 0x6c, 0x75, 0x65, 0x03, 0x7f, 0x01, 0x1c, 0x2e, 0x2f,
        0x6d, 0x75, 0x74, 0x61, 0x62, 0x6c, 0x65, 0x2d, 0x67, 0x6c, 0x6f, 0x62, 0x61, 0x6c, 0x2d,
        0x65, 0x78, 0x70, 0x6f, 0x72, 0x74, 0x2e, 0x77, 0x61, 0x73, 0x6d, 0x0a, 0x76, 0x31, 0x32,
        0x38, 0x45, 0x78, 0x70, 0x6f, 0x72, 0x74, 0x03, 0x7b, 0x01, 0x03, 0x04, 0x03, 0x00, 0x01,
        0x02, 0x07, 0x6f, 0x05, 0x16, 0x72, 0x65, 0x65, 0x78, 0x70, 0x6f, 0x72, 0x74, 0x65, 0x64,
        0x4d, 0x75, 0x74, 0x61, 0x62, 0x6c, 0x65, 0x56, 0x61, 0x6c, 0x75, 0x65, 0x03, 0x00, 0x14,
        0x72, 0x65, 0x65, 0x78, 0x70, 0x6f, 0x72, 0x74, 0x65, 0x64, 0x56, 0x31, 0x32, 0x38, 0x45,
        0x78, 0x70, 0x6f, 0x72, 0x74, 0x03, 0x01, 0x11, 0x73, 0x65, 0x74, 0x49, 0x6d, 0x70, 0x6f,
        0x72, 0x74, 0x65, 0x64, 0x47, 0x6c, 0x6f, 0x62, 0x61, 0x6c, 0x00, 0x00, 0x11, 0x67, 0x65,
        0x74, 0x49, 0x6d, 0x70, 0x6f, 0x72, 0x74, 0x65, 0x64, 0x47, 0x6c, 0x6f, 0x62, 0x61, 0x6c,
        0x00, 0x01, 0x13, 0x67, 0x65, 0x74, 0x49, 0x6d, 0x70, 0x6f, 0x72, 0x74, 0x65, 0x64, 0x56,
        0x31, 0x32, 0x38, 0x4c, 0x61, 0x6e, 0x65, 0x00, 0x02, 0x0a, 0x3f, 0x03, 0x06, 0x00, 0x20,
        0x00, 0x24, 0x00, 0x0b, 0x04, 0x00, 0x23, 0x00, 0x0b, 0x31, 0x00, 0x20, 0x00, 0x41, 0x00,
        0x46, 0x04, 0x7f, 0x23, 0x01, 0xfd, 0x1b, 0x00, 0x05, 0x20, 0x00, 0x41, 0x01, 0x46, 0x04,
        0x7f, 0x23, 0x01, 0xfd, 0x1b, 0x01, 0x05, 0x20, 0x00, 0x41, 0x02, 0x46, 0x04, 0x7f, 0x23,
        0x01, 0xfd, 0x1b, 0x02, 0x05, 0x23, 0x01, 0xfd, 0x1b, 0x03, 0x0b, 0x0b, 0x0b, 0x0b, 0x00,
        0x7f, 0x04, 0x6e, 0x61, 0x6d, 0x65, 0x01, 0x3c, 0x03, 0x00, 0x11, 0x73, 0x65, 0x74, 0x49,
        0x6d, 0x70, 0x6f, 0x72, 0x74, 0x65, 0x64, 0x47, 0x6c, 0x6f, 0x62, 0x61, 0x6c, 0x01, 0x11,
        0x67, 0x65, 0x74, 0x49, 0x6d, 0x70, 0x6f, 0x72, 0x74, 0x65, 0x64, 0x47, 0x6c, 0x6f, 0x62,
        0x61, 0x6c, 0x02, 0x13, 0x67, 0x65, 0x74, 0x49, 0x6d, 0x70, 0x6f, 0x72, 0x74, 0x65, 0x64,
        0x56, 0x31, 0x32, 0x38, 0x4c, 0x61, 0x6e, 0x65, 0x02, 0x16, 0x02, 0x00, 0x01, 0x00, 0x09,
        0x6e, 0x65, 0x77, 0x5f, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x02, 0x01, 0x00, 0x04, 0x6c, 0x61,
        0x6e, 0x65, 0x07, 0x22, 0x02, 0x00, 0x10, 0x69, 0x6d, 0x70, 0x6f, 0x72, 0x74, 0x65, 0x64,
        0x5f, 0x6d, 0x75, 0x74, 0x61, 0x62, 0x6c, 0x65, 0x01, 0x0d, 0x69, 0x6d, 0x70, 0x6f, 0x72,
        0x74, 0x65, 0x64, 0x5f, 0x76, 0x31, 0x32, 0x38,
    ];
    (
        [(CONTENT_TYPE, "application/wasm")],
        MUTABLE_GLOBAL_REEXPORT_WASM.to_vec(),
    )
        .into_response()
}

pub(super) async fn asset_execute_start_wasm() -> Response {
    const EXECUTE_START_WASM: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x02,
        0x14, 0x01, 0x08, 0x2e, 0x2f, 0x6c, 0x6f, 0x67, 0x2e, 0x6a, 0x73, 0x07, 0x6c, 0x6f, 0x67,
        0x45, 0x78, 0x65, 0x63, 0x00, 0x00, 0x03, 0x02, 0x01, 0x00, 0x08, 0x01, 0x01, 0x0a, 0x06,
        0x01, 0x04, 0x00, 0x10, 0x00, 0x0b,
    ];
    (
        [(CONTENT_TYPE, "application/wasm")],
        EXECUTE_START_WASM.to_vec(),
    )
        .into_response()
}

pub(super) async fn asset_js_wasm_cycle_function_error_module() -> Response {
    javascript_response(
        r#"export const func = 42;
import { f } from "./js-wasm-cycle-function-error.wasm";"#,
    )
}

pub(super) async fn asset_module_source_phase_identity_js() -> Response {
    javascript_response(
        r#"import * as namespace1 from "./module-wasm-exported-names.wasm";
import * as namespace2 from "./module-wasm-exported-names.wasm";
import source source1 from "./module-wasm-exported-names.wasm";
import source source2 from "./module-wasm-exported-names.wasm";

export const namespaceShared = namespace1 === namespace2;
export const sourceShared = source1 === source2;"#,
    )
}

pub(super) async fn asset_js_wasm_cycle_function_error_wasm() -> Response {
    const JS_WASM_CYCLE_FUNCTION_ERROR_WASM: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x02,
        0x2a, 0x01, 0x21, 0x2e, 0x2f, 0x6a, 0x73, 0x2d, 0x77, 0x61, 0x73, 0x6d, 0x2d, 0x63, 0x79,
        0x63, 0x6c, 0x65, 0x2d, 0x66, 0x75, 0x6e, 0x63, 0x74, 0x69, 0x6f, 0x6e, 0x2d, 0x65, 0x72,
        0x72, 0x6f, 0x72, 0x2e, 0x6a, 0x73, 0x04, 0x66, 0x75, 0x6e, 0x63, 0x00, 0x00, 0x03, 0x02,
        0x01, 0x00, 0x07, 0x05, 0x01, 0x01, 0x66, 0x00, 0x01, 0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b,
    ];
    (
        [(CONTENT_TYPE, "application/wasm")],
        JS_WASM_CYCLE_FUNCTION_ERROR_WASM.to_vec(),
    )
        .into_response()
}

pub(super) async fn asset_document_wasm_js_cycle_entry_module() -> Response {
    javascript_response(
        r#"import "./document-wasm-js-cycle.wasm";
window.moduleWasmJsCycleUnexpected = true;"#,
    )
}

pub(super) async fn asset_document_wasm_js_cycle_module() -> Response {
    const DOCUMENT_WASM_JS_CYCLE: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x02,
        0x10, 0x01, 0x0a, 0x2e, 0x2f, 0x63, 0x79, 0x63, 0x6c, 0x65, 0x2e, 0x6a, 0x73, 0x01, 0x66,
        0x00, 0x00, 0x03, 0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6e, 0x00, 0x01,
        0x0a, 0x06, 0x01, 0x04, 0x00, 0x10, 0x00, 0x0b,
    ];
    (
        [(CONTENT_TYPE, "application/wasm")],
        DOCUMENT_WASM_JS_CYCLE.to_vec(),
    )
        .into_response()
}

pub(super) async fn asset_wasm_js_cycle_module() -> Response {
    // Derived from WPT wasm/webapi/esm-integration/resources/wasm-js-cycle.wasm.
    // It imports JS-created global/memory/table/function values from
    // ./wasm-js-cycle.js and exports wasm functions that prove the instance
    // captured those values before the JS module later mutates its bindings.
    const WASM_JS_CYCLE: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x09, 0x02, 0x60, 0x00, 0x01, 0x7f,
        0x60, 0x00, 0x01, 0x70, 0x02, 0x73, 0x04, 0x12, 0x2e, 0x2f, 0x77, 0x61, 0x73, 0x6d, 0x2d,
        0x6a, 0x73, 0x2d, 0x63, 0x79, 0x63, 0x6c, 0x65, 0x2e, 0x6a, 0x73, 0x06, 0x6a, 0x73, 0x47,
        0x6c, 0x6f, 0x62, 0x03, 0x7f, 0x01, 0x12, 0x2e, 0x2f, 0x77, 0x61, 0x73, 0x6d, 0x2d, 0x6a,
        0x73, 0x2d, 0x63, 0x79, 0x63, 0x6c, 0x65, 0x2e, 0x6a, 0x73, 0x05, 0x6a, 0x73, 0x4d, 0x65,
        0x6d, 0x02, 0x00, 0x0a, 0x12, 0x2e, 0x2f, 0x77, 0x61, 0x73, 0x6d, 0x2d, 0x6a, 0x73, 0x2d,
        0x63, 0x79, 0x63, 0x6c, 0x65, 0x2e, 0x6a, 0x73, 0x05, 0x6a, 0x73, 0x54, 0x61, 0x62, 0x01,
        0x70, 0x00, 0x0a, 0x12, 0x2e, 0x2f, 0x77, 0x61, 0x73, 0x6d, 0x2d, 0x6a, 0x73, 0x2d, 0x63,
        0x79, 0x63, 0x6c, 0x65, 0x2e, 0x6a, 0x73, 0x06, 0x6a, 0x73, 0x46, 0x75, 0x6e, 0x63, 0x00,
        0x00, 0x03, 0x05, 0x04, 0x00, 0x00, 0x01, 0x00, 0x06, 0x06, 0x01, 0x7f, 0x00, 0x41, 0x18,
        0x0b, 0x07, 0x53, 0x07, 0x08, 0x77, 0x61, 0x73, 0x6d, 0x47, 0x6c, 0x6f, 0x62, 0x03, 0x01,
        0x07, 0x77, 0x61, 0x73, 0x6d, 0x4d, 0x65, 0x6d, 0x02, 0x00, 0x07, 0x77, 0x61, 0x73, 0x6d,
        0x54, 0x61, 0x62, 0x01, 0x00, 0x0d, 0x69, 0x6e, 0x63, 0x72, 0x65, 0x6d, 0x65, 0x6e, 0x74,
        0x47, 0x6c, 0x6f, 0x62, 0x00, 0x01, 0x09, 0x6d, 0x75, 0x74, 0x61, 0x74, 0x65, 0x4d, 0x65,
        0x6d, 0x00, 0x02, 0x09, 0x6d, 0x75, 0x74, 0x61, 0x74, 0x65, 0x54, 0x61, 0x62, 0x00, 0x03,
        0x08, 0x77, 0x61, 0x73, 0x6d, 0x46, 0x75, 0x6e, 0x63, 0x00, 0x04, 0x09, 0x05, 0x01, 0x03,
        0x00, 0x01, 0x04, 0x0a, 0x31, 0x04, 0x0b, 0x00, 0x41, 0x01, 0x23, 0x00, 0x6a, 0x24, 0x00,
        0x23, 0x00, 0x0b, 0x0e, 0x00, 0x41, 0x00, 0x41, 0x2a, 0x36, 0x02, 0x00, 0x41, 0x00, 0x28,
        0x02, 0x00, 0x0b, 0x0c, 0x00, 0x41, 0x00, 0xd2, 0x04, 0x26, 0x00, 0x41, 0x00, 0x25, 0x00,
        0x0b, 0x07, 0x00, 0x10, 0x00, 0x41, 0x01, 0x6a, 0x0b,
    ];
    ([(CONTENT_TYPE, "application/wasm")], WASM_JS_CYCLE.to_vec()).into_response()
}

pub(super) async fn asset_wasm_js_cycle_dependency_module() -> Response {
    javascript_response(
        r#"import * as mod from "./wasm-js-cycle.wasm";

let jsGlob = new WebAssembly.Global({ value: "i32", mutable: true }, 42);
let jsMem = new WebAssembly.Memory({ initial: 10 });
let jsTab = new WebAssembly.Table({ initial: 10, element: "anyfunc" });
let jsFunc = () => 42;

export { jsFunc, jsGlob, jsMem, jsTab };

export function mutateBindings() {
  jsGlob = 0;
  jsMem = 0;
  jsTab = 0;
  jsFunc = 0;
}"#,
    )
}

pub(super) async fn asset_document_js_wasm_cycle_entry_module() -> Response {
    javascript_response(
        r#"export function f() {
  return 42;
}

import { run } from "./js-higher-cycle.wasm";

window.moduleJsWasmCycleInitialRun = run();
f = () => 24;
window.moduleJsWasmCycleAfterReassignRun = run();"#,
    )
}

pub(super) async fn asset_document_js_wasm_cycle_module() -> Response {
    // (module
    //   (import "./jscyc.js" "f" (func $f (result i32)))
    //   (func (export "run") (result i32) call $f))
    const DOCUMENT_JS_WASM_CYCLE: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f,
        0x02, 0x10, 0x01, 0x0a, 0x2e, 0x2f, 0x6a, 0x73, 0x63, 0x79, 0x63, 0x2e, 0x6a, 0x73, 0x01,
        0x66, 0x00, 0x00, 0x03, 0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6e, 0x00,
        0x01, 0x0a, 0x06, 0x01, 0x04, 0x00, 0x10, 0x00, 0x0b,
    ];
    (
        [(CONTENT_TYPE, "application/wasm")],
        DOCUMENT_JS_WASM_CYCLE.to_vec(),
    )
        .into_response()
}

pub(super) async fn asset_document_wasm_js_cycle_dependency_module() -> Response {
    javascript_response(
        r#"import * as wasm from "./document-wasm-js-cycle.wasm";
export function f() {
  return wasm.run();
}"#,
    )
}

pub(super) async fn asset_invalid_wasm_import_name() -> Response {
    const INVALID_WASM_IMPORT_NAME: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f,
        0x02, 0x15, 0x01, 0x04, 0x74, 0x65, 0x73, 0x74, 0x0c, 0x77, 0x61, 0x73, 0x6d, 0x3a, 0x69,
        0x6e, 0x76, 0x61, 0x6c, 0x69, 0x64, 0x00, 0x00, 0x03, 0x02, 0x01, 0x00, 0x07, 0x08, 0x01,
        0x04, 0x74, 0x65, 0x73, 0x74, 0x00, 0x01, 0x0a, 0x06, 0x01, 0x04, 0x00, 0x10, 0x00, 0x0b,
        0x00, 0x1d, 0x04, 0x6e, 0x61, 0x6d, 0x65, 0x01, 0x16, 0x02, 0x00, 0x0d, 0x69, 0x6e, 0x76,
        0x61, 0x6c, 0x69, 0x64, 0x49, 0x6d, 0x70, 0x6f, 0x72, 0x74, 0x01, 0x04, 0x74, 0x65, 0x73,
        0x74,
    ];
    (
        [(CONTENT_TYPE, "application/wasm")],
        INVALID_WASM_IMPORT_NAME.to_vec(),
    )
        .into_response()
}

pub(super) async fn asset_invalid_wasm_export_name() -> Response {
    const INVALID_WASM_EXPORT_NAME: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f,
        0x03, 0x02, 0x01, 0x00, 0x07, 0x10, 0x01, 0x0c, 0x77, 0x61, 0x73, 0x6d, 0x3a, 0x69, 0x6e,
        0x76, 0x61, 0x6c, 0x69, 0x64, 0x00, 0x00, 0x0a, 0x06, 0x01, 0x04, 0x00, 0x41, 0x2a, 0x0b,
        0x00, 0x0e, 0x04, 0x6e, 0x61, 0x6d, 0x65, 0x01, 0x07, 0x01, 0x00, 0x04, 0x74, 0x65, 0x73,
        0x74,
    ];
    (
        [(CONTENT_TYPE, "application/wasm")],
        INVALID_WASM_EXPORT_NAME.to_vec(),
    )
        .into_response()
}

pub(super) async fn asset_invalid_wasm_import_module() -> Response {
    const INVALID_WASM_IMPORT_MODULE: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f,
        0x02, 0x18, 0x01, 0x0f, 0x77, 0x61, 0x73, 0x6d, 0x2d, 0x6a, 0x73, 0x3a, 0x69, 0x6e, 0x76,
        0x61, 0x6c, 0x69, 0x64, 0x04, 0x74, 0x65, 0x73, 0x74, 0x00, 0x00, 0x03, 0x02, 0x01, 0x00,
        0x07, 0x08, 0x01, 0x04, 0x74, 0x65, 0x73, 0x74, 0x00, 0x01, 0x0a, 0x06, 0x01, 0x04, 0x00,
        0x10, 0x00, 0x0b, 0x00, 0x1d, 0x04, 0x6e, 0x61, 0x6d, 0x65, 0x01, 0x16, 0x02, 0x00, 0x0d,
        0x69, 0x6e, 0x76, 0x61, 0x6c, 0x69, 0x64, 0x49, 0x6d, 0x70, 0x6f, 0x72, 0x74, 0x01, 0x04,
        0x74, 0x65, 0x73, 0x74,
    ];
    (
        [(CONTENT_TYPE, "application/wasm")],
        INVALID_WASM_IMPORT_MODULE.to_vec(),
    )
        .into_response()
}

pub(super) async fn asset_module_import_assertions_legacy_barrel() -> Response {
    javascript_response(MODULE_IMPORT_ASSERTIONS_LEGACY_BARREL_MJS)
}

pub(super) async fn asset_module_dynamic_import_template_target() -> Response {
    javascript_response(MODULE_DYNAMIC_IMPORT_TEMPLATE_TARGET_MJS)
}

pub(super) async fn asset_module_dynamic_import_string_compilation_target() -> Response {
    javascript_response(
        r#"export const label = "dynamic-base-ok";
export const urlSuffix = import.meta.url.endsWith("?eval")
  ? "eval"
  : import.meta.url.endsWith("?function")
    ? "function"
    : import.meta.url;
"#,
    )
}

pub(super) async fn asset_module_escaped_string_literal_specifiers_source() -> Response {
    javascript_response(MODULE_ESCAPED_STRING_LITERAL_SPECIFIERS_SOURCE_MJS)
}

pub(super) async fn asset_module_escaped_string_literal_specifiers_barrel() -> Response {
    javascript_response(MODULE_ESCAPED_STRING_LITERAL_SPECIFIERS_BARREL_MJS)
}

pub(super) async fn asset_module_multiline_source() -> Response {
    javascript_response(MODULE_MULTILINE_SOURCE_MJS)
}

pub(super) async fn asset_module_multiline_barrel() -> Response {
    javascript_response(MODULE_MULTILINE_BARREL_MJS)
}

pub(super) async fn asset_module_export_star_source() -> Response {
    javascript_response(MODULE_EXPORT_STAR_SOURCE_MJS)
}

pub(super) async fn asset_module_export_star_barrel() -> Response {
    javascript_response(MODULE_EXPORT_STAR_BARREL_MJS)
}

pub(super) async fn asset_module_export_star_ambiguous_a() -> Response {
    javascript_response(MODULE_EXPORT_STAR_AMBIGUOUS_A_MJS)
}

pub(super) async fn asset_module_export_star_ambiguous_b() -> Response {
    javascript_response(MODULE_EXPORT_STAR_AMBIGUOUS_B_MJS)
}

pub(super) async fn asset_module_export_star_ambiguous_barrel() -> Response {
    javascript_response(MODULE_EXPORT_STAR_AMBIGUOUS_BARREL_MJS)
}

pub(super) async fn asset_module_cycle_missing_export_a() -> Response {
    javascript_response(MODULE_CYCLE_MISSING_EXPORT_A_MJS)
}

pub(super) async fn asset_module_cycle_missing_export_b() -> Response {
    javascript_response(MODULE_CYCLE_MISSING_EXPORT_B_MJS)
}

pub(super) async fn asset_module_cycle_initializing_missing_export_a() -> Response {
    javascript_response(MODULE_CYCLE_INITIALIZING_MISSING_EXPORT_A_MJS)
}

pub(super) async fn asset_module_cycle_initializing_missing_export_b() -> Response {
    javascript_response(MODULE_CYCLE_INITIALIZING_MISSING_EXPORT_B_MJS)
}

pub(super) async fn asset_module_cycle_default_missing_a() -> Response {
    javascript_response(MODULE_CYCLE_DEFAULT_MISSING_A_MJS)
}

pub(super) async fn asset_module_cycle_default_missing_b() -> Response {
    javascript_response(MODULE_CYCLE_DEFAULT_MISSING_B_MJS)
}

pub(super) async fn asset_module_cycle_dynamic_import_waits_a() -> Response {
    javascript_response(MODULE_CYCLE_DYNAMIC_IMPORT_WAITS_A_MJS)
}

pub(super) async fn asset_module_cycle_dynamic_import_waits_b() -> Response {
    javascript_response(MODULE_CYCLE_DYNAMIC_IMPORT_WAITS_B_MJS)
}

pub(super) async fn asset_module_cycle_export_star_late_barrel() -> Response {
    javascript_response(MODULE_CYCLE_EXPORT_STAR_LATE_BARREL_MJS)
}

pub(super) async fn asset_module_cycle_export_star_late_source() -> Response {
    javascript_response(MODULE_CYCLE_EXPORT_STAR_LATE_SOURCE_MJS)
}

pub(super) async fn asset_module_cycle_export_star_multihop_outer_barrel() -> Response {
    javascript_response(MODULE_CYCLE_EXPORT_STAR_MULTIHOP_OUTER_BARREL_MJS)
}

pub(super) async fn asset_module_cycle_export_star_multihop_inner_barrel() -> Response {
    javascript_response(MODULE_CYCLE_EXPORT_STAR_MULTIHOP_INNER_BARREL_MJS)
}

pub(super) async fn asset_module_cycle_export_star_multihop_source() -> Response {
    javascript_response(MODULE_CYCLE_EXPORT_STAR_MULTIHOP_SOURCE_MJS)
}

pub(super) async fn asset_module_cycle_export_star_late_ambiguous_barrel() -> Response {
    javascript_response(MODULE_CYCLE_EXPORT_STAR_LATE_AMBIGUOUS_BARREL_MJS)
}

pub(super) async fn asset_module_cycle_export_star_late_ambiguous_a() -> Response {
    javascript_response(MODULE_CYCLE_EXPORT_STAR_LATE_AMBIGUOUS_A_MJS)
}

pub(super) async fn asset_module_cycle_export_star_late_ambiguous_b() -> Response {
    javascript_response(MODULE_CYCLE_EXPORT_STAR_LATE_AMBIGUOUS_B_MJS)
}

pub(super) async fn asset_module_cycle_export_star_multihop_late_ambiguous_outer_barrel() -> Response
{
    javascript_response(MODULE_CYCLE_EXPORT_STAR_MULTIHOP_LATE_AMBIGUOUS_OUTER_BARREL_MJS)
}

pub(super) async fn asset_module_cycle_export_star_multihop_late_ambiguous_inner_barrel() -> Response
{
    javascript_response(MODULE_CYCLE_EXPORT_STAR_MULTIHOP_LATE_AMBIGUOUS_INNER_BARREL_MJS)
}

pub(super) async fn asset_module_cycle_export_star_multihop_late_ambiguous_a() -> Response {
    javascript_response(MODULE_CYCLE_EXPORT_STAR_MULTIHOP_LATE_AMBIGUOUS_A_MJS)
}

pub(super) async fn asset_module_cycle_export_star_multihop_late_ambiguous_b() -> Response {
    javascript_response(MODULE_CYCLE_EXPORT_STAR_MULTIHOP_LATE_AMBIGUOUS_B_MJS)
}

pub(super) async fn asset_module_pending_star_cycle_entry() -> Response {
    javascript_response(MODULE_PENDING_STAR_CYCLE_ENTRY_MJS)
}

pub(super) async fn asset_module_pending_star_cycle_a() -> Response {
    javascript_response(MODULE_PENDING_STAR_CYCLE_A_MJS)
}

pub(super) async fn asset_module_pending_star_cycle_b() -> Response {
    javascript_response(MODULE_PENDING_STAR_CYCLE_B_MJS)
}

pub(super) async fn asset_module_pending_star_cycle_c() -> Response {
    javascript_response(MODULE_PENDING_STAR_CYCLE_C_MJS)
}

pub(super) async fn asset_module_pending_star_body_cycle_a() -> Response {
    javascript_response(MODULE_PENDING_STAR_BODY_CYCLE_A_MJS)
}

pub(super) async fn asset_module_pending_star_body_cycle_b() -> Response {
    javascript_response(MODULE_PENDING_STAR_BODY_CYCLE_B_MJS)
}

pub(super) async fn asset_module_shared_initializing_dep() -> Response {
    javascript_response(MODULE_SHARED_INITIALIZING_DEP_MJS)
}

pub(super) async fn asset_module_shared_initializing_parent_a() -> Response {
    javascript_response(MODULE_SHARED_INITIALIZING_PARENT_A_MJS)
}

pub(super) async fn asset_module_shared_initializing_parent_b() -> Response {
    javascript_response(MODULE_SHARED_INITIALIZING_PARENT_B_MJS)
}

pub(super) async fn asset_module_shared_failed_dep() -> Response {
    javascript_response(MODULE_SHARED_FAILED_DEP_MJS)
}

pub(super) async fn asset_module_shared_failed_parent_a() -> Response {
    javascript_response(MODULE_SHARED_FAILED_PARENT_A_MJS)
}

pub(super) async fn asset_module_shared_failed_parent_b() -> Response {
    javascript_response(MODULE_SHARED_FAILED_PARENT_B_MJS)
}

pub(super) async fn asset_module_shared_unsupported_dep() -> Response {
    javascript_response(MODULE_SHARED_UNSUPPORTED_DEP_MJS)
}

pub(super) async fn asset_module_shared_unsupported_parent_a() -> Response {
    javascript_response(MODULE_SHARED_UNSUPPORTED_PARENT_A_MJS)
}

pub(super) async fn asset_module_shared_unsupported_parent_b() -> Response {
    javascript_response(MODULE_SHARED_UNSUPPORTED_PARENT_B_MJS)
}

pub(super) async fn asset_module_link_exports_only_named() -> Response {
    javascript_response(MODULE_LINK_EXPORTS_ONLY_NAMED_MJS)
}

pub(super) async fn asset_module_side_effect_only() -> Response {
    javascript_response(MODULE_SIDE_EFFECT_ONLY_MJS)
}

pub(super) async fn asset_module_default_function_export() -> Response {
    javascript_response(MODULE_DEFAULT_FUNCTION_EXPORT_MJS)
}

pub(super) async fn asset_module_default_class_export() -> Response {
    javascript_response(MODULE_DEFAULT_CLASS_EXPORT_MJS)
}

pub(super) async fn asset_module_default_anonymous_function_export() -> Response {
    javascript_response(MODULE_DEFAULT_ANONYMOUS_FUNCTION_EXPORT_MJS)
}

pub(super) async fn asset_module_default_anonymous_class_export() -> Response {
    javascript_response(MODULE_DEFAULT_ANONYMOUS_CLASS_EXPORT_MJS)
}

pub(super) async fn asset_module_export_class_named() -> Response {
    javascript_response(MODULE_EXPORT_CLASS_NAMED_MJS)
}

pub(super) async fn asset_module_export_generator_functions() -> Response {
    javascript_response(MODULE_EXPORT_GENERATOR_FUNCTIONS_MJS)
}

pub(super) async fn asset_module_export_const_multiple_bindings() -> Response {
    javascript_response(MODULE_EXPORT_CONST_MULTIPLE_BINDINGS_MJS)
}

pub(super) async fn asset_module_multiline_dynamic_import_target() -> Response {
    javascript_response(MODULE_MULTILINE_DYNAMIC_IMPORT_TARGET_MJS)
}

pub(super) async fn asset_module_export_variable_live_bindings() -> Response {
    javascript_response(MODULE_EXPORT_VARIABLE_LIVE_BINDINGS_MJS)
}

pub(super) async fn asset_module_self_bare_dynamic_import_resolves_after_own_evaluation() -> Response
{
    javascript_response(MODULE_SELF_BARE_DYNAMIC_IMPORT_RESOLVES_AFTER_OWN_EVALUATION_MJS)
}

pub(super) async fn asset_module_self_bare_dynamic_import_after_settle_resolves() -> Response {
    javascript_response(MODULE_SELF_BARE_DYNAMIC_IMPORT_AFTER_SETTLE_RESOLVES_MJS)
}

pub(super) async fn asset_module_runtime_helper_shadowing() -> Response {
    javascript_response(MODULE_RUNTIME_HELPER_SHADOWING_MJS)
}

pub(super) async fn asset_module_runtime_helper_shadowing_source() -> Response {
    javascript_response(MODULE_RUNTIME_HELPER_SHADOWING_SOURCE_MJS)
}

pub(super) async fn asset_dynamic_async_module_acquisition_barrier() -> Response {
    sleep(Duration::from_millis(40)).await;
    javascript_response(DYNAMIC_ASYNC_MODULE_ACQUISITION_BARRIER_MJS)
}

pub(super) async fn asset_module_pkg_entry() -> Response {
    javascript_response(MODULE_PKG_ENTRY_MJS)
}

pub(super) async fn asset_module_pkg_scoped_entry() -> Response {
    javascript_response(MODULE_PKG_SCOPED_ENTRY_MJS)
}

pub(super) async fn asset_blocking_stylesheet_slow_css() -> Response {
    sleep(Duration::from_millis(75)).await;
    css_response(BLOCKING_STYLESHEET_SLOW_CSS)
}

pub(super) async fn asset_dynamic_blocking_stylesheet_gated_css(
    Extension(state): Extension<FixtureRuntimeState>,
) -> Response {
    state.dynamic_stylesheet_script_executed.wait().await;
    css_response(BLOCKING_STYLESHEET_SLOW_CSS)
}

pub(super) async fn asset_runtime_connected_preload_very_slow_css() -> Response {
    sleep(Duration::from_millis(250)).await;
    css_response(BLOCKING_STYLESHEET_SLOW_CSS)
}

pub(super) async fn asset_runtime_connected_modulepreload_slow_module() -> Response {
    sleep(Duration::from_millis(75)).await;
    javascript_response(RUNTIME_CONNECTED_MODULEPRELOAD_SLOW_MJS)
}

pub(super) async fn asset_runtime_connected_modulepreload_very_slow_module() -> Response {
    sleep(Duration::from_millis(250)).await;
    javascript_response(RUNTIME_CONNECTED_MODULEPRELOAD_SLOW_MJS)
}

pub(super) async fn asset_modulepreload_shared_root_module() -> Response {
    javascript_response(MODULEPRELOAD_SHARED_ROOT_MJS)
}

pub(super) async fn asset_modulepreload_shared_mid_module() -> Response {
    javascript_response(MODULEPRELOAD_SHARED_MID_MJS)
}

pub(super) async fn asset_modulepreload_shared_leaf_slow_module() -> Response {
    sleep(Duration::from_millis(75)).await;
    javascript_response(MODULEPRELOAD_SHARED_LEAF_SLOW_MJS)
}

pub(super) async fn asset_modulepreload_duplicate_root_module() -> Response {
    javascript_response(MODULEPRELOAD_DUPLICATE_ROOT_MJS)
}

pub(super) async fn asset_modulepreload_duplicate_parent_a_module() -> Response {
    javascript_response(MODULEPRELOAD_DUPLICATE_PARENT_A_MJS)
}

pub(super) async fn asset_modulepreload_duplicate_parent_b_module() -> Response {
    javascript_response(MODULEPRELOAD_DUPLICATE_PARENT_B_MJS)
}

pub(super) async fn asset_modulepreload_duplicate_leaf_slow_module() -> Response {
    sleep(Duration::from_millis(75)).await;
    javascript_response(MODULEPRELOAD_DUPLICATE_LEAF_SLOW_MJS)
}

pub(super) async fn asset_duplicate_module_root_eval_module() -> Response {
    javascript_response(DUPLICATE_MODULE_ROOT_EVAL_MJS)
}

pub(super) async fn asset_duplicate_nested_this_module() -> Response {
    javascript_response(DUPLICATE_NESTED_THIS_MJS)
}

pub(super) async fn asset_duplicate_nested_this_nested_module() -> Response {
    javascript_response(DUPLICATE_NESTED_THIS_NESTED_MJS)
}

pub(super) async fn asset_module_wrong_mime_css() -> Response {
    Response::builder()
        .header(CONTENT_TYPE, HeaderValue::from_static("text/css"))
        .body(Body::from(MODULE_WRONG_MIME_CSS))
        .expect("build wrong MIME module response")
}

pub(super) async fn asset_modulepreload_reused_root_slow_module() -> Response {
    sleep(Duration::from_millis(75)).await;
    javascript_response(MODULEPRELOAD_REUSED_ROOT_SLOW_MJS)
}

pub(super) async fn asset_modulepreload_reused_parent_module() -> Response {
    javascript_response(MODULEPRELOAD_REUSED_PARENT_MJS)
}

pub(super) async fn asset_modulepreload_reused_child_slow_module() -> Response {
    sleep(Duration::from_millis(250)).await;
    javascript_response(MODULEPRELOAD_REUSED_CHILD_SLOW_MJS)
}

pub(super) async fn asset_shadow_adopted_modulepreload_css() -> Response {
    css_response(SHADOW_ADOPTED_MODULEPRELOAD_CSS)
}

pub(super) async fn redirect_page() -> Redirect {
    Redirect::temporary("/static")
}

pub(super) async fn redirect_cookie_page() -> Response {
    (
        [(SET_COOKIE, "session=fixture; Path=/; HttpOnly")],
        Redirect::temporary("/cookie"),
    )
        .into_response()
}

pub(super) async fn cookie_page(headers: HeaderMap) -> Response {
    let has_cookie = headers
        .get(COOKIE)
        .and_then(|value| value.to_str().ok())
        .map(|cookies| cookies.contains("session=fixture"))
        .unwrap_or(false);

    if has_cookie {
        Html(COOKIE_SEEN_HTML).into_response()
    } else {
        (
            [(SET_COOKIE, "session=fixture; Path=/; HttpOnly")],
            Html(COOKIE_MISSING_HTML),
        )
            .into_response()
    }
}

pub(super) async fn cookie_location_gate_page(headers: HeaderMap) -> Response {
    // This mirrors Toutiao's first-visit shape: absent visitor cookie, page JS
    // writes a browser-visible id cookie and rewrites the URL with a wid query.
    if has_cookie(&headers, "ttwid=fixture") {
        Html(COOKIE_LOCATION_GATE_SEEN_HTML).into_response()
    } else {
        Html(COOKIE_LOCATION_GATE_MISSING_HTML).into_response()
    }
}

pub(super) async fn cookie_scope_set_page() -> Response {
    (
        [(SET_COOKIE, "scope=match; Path=/cookie-scope; HttpOnly")],
        Html(COOKIE_SCOPE_MISSING_HTML),
    )
        .into_response()
}

pub(super) async fn cookie_scope_check_page(headers: HeaderMap) -> Response {
    if has_cookie(&headers, "scope=match") {
        Html(COOKIE_SCOPE_SEEN_HTML).into_response()
    } else {
        Html(COOKIE_SCOPE_MISSING_HTML).into_response()
    }
}

pub(super) async fn cookie_scope_extra_check_page(headers: HeaderMap) -> Response {
    if has_cookie(&headers, "scope=match") {
        Html(COOKIE_SCOPE_SEEN_HTML).into_response()
    } else {
        Html(COOKIE_SCOPE_MISSING_HTML).into_response()
    }
}

pub(super) async fn cookie_invalid_domain_set_page() -> Response {
    (
        [(SET_COOKIE, "bad=1; Domain=example.com; Path=/; HttpOnly")],
        Html(COOKIE_DOMAIN_MISSING_HTML),
    )
        .into_response()
}

pub(super) async fn cookie_invalid_domain_check_page(headers: HeaderMap) -> Response {
    if has_cookie(&headers, "bad=1") {
        Html(COOKIE_DOMAIN_SEEN_HTML).into_response()
    } else {
        Html(COOKIE_DOMAIN_MISSING_HTML).into_response()
    }
}

pub(super) async fn cookie_replace_red_page() -> Response {
    (
        [(SET_COOKIE, "color=red; Path=/cookie-replace; HttpOnly")],
        Html(COOKIE_REPLACE_RED_HTML),
    )
        .into_response()
}

pub(super) async fn cookie_replace_blue_page() -> Response {
    (
        [(SET_COOKIE, "color=blue; Path=/cookie-replace; HttpOnly")],
        Html(COOKIE_REPLACE_BLUE_HTML),
    )
        .into_response()
}

pub(super) async fn cookie_replace_check_page(headers: HeaderMap) -> Response {
    let body = if has_cookie(&headers, "color=blue") {
        COOKIE_REPLACE_BLUE_HTML
    } else if has_cookie(&headers, "color=red") {
        COOKIE_REPLACE_RED_HTML
    } else {
        COOKIE_MISSING_HTML
    };

    Html(body).into_response()
}

pub(super) async fn redirect_cookie_chain_start() -> Response {
    redirect_with_cookies(
        "/redirect-cookie-chain/middle",
        &[
            "chain=start; Path=/redirect-cookie-chain; HttpOnly",
            "common=one; Path=/; HttpOnly",
        ],
    )
}

pub(super) async fn redirect_cookie_chain_middle(headers: HeaderMap) -> Response {
    if !has_cookie(&headers, "chain=start") || !has_cookie(&headers, "common=one") {
        return Html(COOKIE_CHAIN_BROKEN_HTML).into_response();
    }

    redirect_with_cookies(
        "/redirect-cookie-chain/final",
        &[
            "common=two; Path=/; HttpOnly",
            "middle=seen; Path=/redirect-cookie-chain; HttpOnly",
        ],
    )
}

pub(super) async fn redirect_cookie_chain_final(headers: HeaderMap) -> Response {
    if has_cookie(&headers, "chain=start")
        && has_cookie(&headers, "common=two")
        && has_cookie(&headers, "middle=seen")
    {
        Html(COOKIE_CHAIN_OK_HTML).into_response()
    } else {
        Html(COOKIE_CHAIN_BROKEN_HTML).into_response()
    }
}

pub(super) async fn net_json_page() -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "application/json")],
        r#"{"ok":true,"value":42}"#,
    )
}

pub(super) async fn net_echo_page(request: AxumRequest) -> impl IntoResponse {
    let method = request.method().as_str().to_owned();
    let x_test = request
        .headers()
        .get("x-test")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let x_trace = request
        .headers()
        .get("x-trace")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let authorization = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let x_empty = request
        .headers()
        .get("x-empty")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let body = format!(
        r#"{{"method":"{method}","received":true,"x-test":"{x_test}","x-trace":"{x_trace}","authorization":"{authorization}","x-empty":"{x_empty}"}}"#
    );
    ([(CONTENT_TYPE, "application/json")], body)
}

pub(super) async fn net_header_scope_page(request: AxumRequest) -> Html<String> {
    let x_test = request
        .headers()
        .get("x-test")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let x_trace = request
        .headers()
        .get("x-trace")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    Html(format!(
        r#"<!doctype html><html><body data-nav-x-test="{x_test}" data-nav-x-trace="{x_trace}"><main id="navigation">header-scope</main><script>
fetch('/net/echo')
  .then(response => response.text())
  .then(payload => {{
    const pre = document.createElement('pre');
    pre.id = 'subrequest';
    pre.textContent = payload;
    document.body.appendChild(pre);
  }});
</script></body></html>"#
    ))
}

pub(super) async fn net_redirect_page() -> Redirect {
    Redirect::to("/net/json")
}

pub(super) async fn net_xhr_page() -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "application/json")],
        r#"{"xhrOk":true,"version":1}"#,
    )
}

pub(super) async fn net_upstream_xhr_page() -> Response {
    let body = "1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890";
    ([(CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response()
}

pub(super) async fn net_upstream_xhr_json_page() -> impl IntoResponse {
    (
        [
            (CONTENT_TYPE, "application/json"),
            (axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        ],
        r#"{"over":"9000!!!","updated_at":1765867200000}"#,
    )
}

pub(super) async fn net_upstream_xhr_redirect_page() -> Redirect {
    Redirect::to("/net/upstream/xhr")
}

pub(super) async fn net_upstream_xhr_404_page() -> Response {
    (StatusCode::NOT_FOUND, "Not Found").into_response()
}

pub(super) async fn net_upstream_xhr_403_challenge_page() -> Response {
    (
        StatusCode::FORBIDDEN,
        Html(
            "<!doctype html><html><body><main id=\"challenge\">forbidden challenge</main></body></html>",
        ),
    )
        .into_response()
}

pub(super) async fn net_upstream_xhr_404_then_200_page() -> Response {
    let request_index = NET_UPSTREAM_XHR_404_THEN_200_REQUESTS.fetch_add(1, Ordering::SeqCst);
    if request_index == 0 {
        return (StatusCode::NOT_FOUND, "first-hit-404").into_response();
    }
    (StatusCode::OK, "second-hit-200").into_response()
}

pub(super) async fn net_upstream_xhr_500_page() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
}

pub(super) async fn net_upstream_xhr_binary_page() -> Response {
    let bytes = [0u8, 0, 1, 2, 0, 0, 9];
    ([(CONTENT_TYPE, "application/octet-stream")], bytes.to_vec()).into_response()
}

pub(super) async fn net_upstream_xhr_empty_page() -> Response {
    ([(CONTENT_TYPE, "text/plain; charset=utf-8")], "").into_response()
}
