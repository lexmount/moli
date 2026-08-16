use super::*;

const WAIT_UNTIL_LIFECYCLE_HTML: &str = "<!doctype html><html><body><script>document.addEventListener('DOMContentLoaded', () => { window.domReady = true; }); window.addEventListener('load', () => { window.loadReady = true; });</script></body></html>";
// `waitUntil: domcontentloaded` returns while the live page can keep running.
// Leave a stable post-DCL gap so cutoff assertions observe the DCL boundary
// instead of racing the later fetch completion under nextest concurrency.
const WAIT_UNTIL_DOMCONTENTLOADED_FETCH_HTML: &str = "<!doctype html><html><body data-state=\"init\"><script>document.addEventListener('DOMContentLoaded', () => { setTimeout(() => { fetch('/wait-until-data').then(r => r.text()).then(text => { document.body.setAttribute('data-state', text); const main = document.createElement('main'); main.id = 'late-dcl'; main.textContent = text; document.body.appendChild(main); }); }, 300); });</script></body></html>";
const WAIT_UNTIL_DOMCONTENTLOADED_RUNTIME_SCRIPT_HTML: &str = "<!doctype html><html><head></head><body><script>window.runtimeOwnedDclInjectedOrder=[];document.addEventListener('DOMContentLoaded',()=>{window.runtimeOwnedInOrderLoadOrder=window.runtimeOwnedDclInjectedOrder;window.runtimeOwnedDclInjectedOrder.push('dcl:'+document.readyState);const script=document.createElement('script');script.async=false;script.src='/assets/runtime_owned_in_order_load.js';script.onload=()=>{window.runtimeOwnedDclInjectedOrder.push('load');window.runtimeOwnedDclInjectedLoadOrder=window.runtimeOwnedDclInjectedOrder.join(',');const main=document.createElement('main');main.id='late-dcl-script';main.textContent='script-loaded';document.body.appendChild(main);};document.head.appendChild(script);window.runtimeOwnedDclInjectedOrder.push('after-append');window.runtimeOwnedDclInjectedDclOrder=window.runtimeOwnedDclInjectedOrder.join(',');});window.addEventListener('load',()=>{window.runtimeOwnedDclInjectedOrder.push('window-load');window.runtimeOwnedDclInjectedFinalOrder=window.runtimeOwnedDclInjectedOrder.join(',');});</script></body></html>";
// Mirrors the shutdown-sensitive CLI shape: a runtime-owned external script is
// inserted at DCL, but its fetch intentionally completes later.
const WAIT_UNTIL_DOMCONTENTLOADED_RUNTIME_SCRIPT_SLOW_HTML: &str = "<!doctype html><html><head></head><body><script>window.runtimeOwnedDclInjectedOrder=[];document.addEventListener('DOMContentLoaded',()=>{window.runtimeOwnedInOrderLoadOrder=window.runtimeOwnedDclInjectedOrder;window.runtimeOwnedDclInjectedOrder.push('dcl:'+document.readyState);const script=document.createElement('script');script.async=false;script.src='/assets/runtime_owned_in_order_load_slow.js';script.onload=()=>{window.runtimeOwnedDclInjectedOrder.push('load');window.runtimeOwnedDclInjectedLoadOrder=window.runtimeOwnedDclInjectedOrder.join(',');const main=document.createElement('main');main.id='late-dcl-script-slow';main.textContent='script-loaded-slow';document.body.appendChild(main);};document.head.appendChild(script);window.runtimeOwnedDclInjectedOrder.push('after-append');window.runtimeOwnedDclInjectedDclOrder=window.runtimeOwnedDclInjectedOrder.join(',');});window.addEventListener('load',()=>{window.runtimeOwnedDclInjectedOrder.push('window-load');window.runtimeOwnedDclInjectedFinalOrder=window.runtimeOwnedDclInjectedOrder.join(',');});</script></body></html>";
const WAIT_UNTIL_DOMCONTENTLOADED_RUNTIME_SCRIPT_VERY_SLOW_HTML: &str = "<!doctype html><html><head></head><body><script>window.runtimeOwnedDclInjectedOrder=[];document.addEventListener('DOMContentLoaded',()=>{window.runtimeOwnedInOrderLoadOrder=window.runtimeOwnedDclInjectedOrder;window.runtimeOwnedDclInjectedOrder.push('dcl:'+document.readyState);const script=document.createElement('script');script.async=false;script.src='/assets/runtime_owned_in_order_load_very_slow.js';script.onload=()=>{window.runtimeOwnedDclInjectedOrder.push('load');window.runtimeOwnedDclInjectedLoadOrder=window.runtimeOwnedDclInjectedOrder.join(',');const main=document.createElement('main');main.id='late-dcl-script-very-slow';main.textContent='script-loaded-very-slow';document.body.appendChild(main);};document.head.appendChild(script);window.runtimeOwnedDclInjectedOrder.push('after-append');window.runtimeOwnedDclInjectedDclOrder=window.runtimeOwnedDclInjectedOrder.join(',');});window.addEventListener('load',()=>{window.runtimeOwnedDclInjectedOrder.push('window-load');window.runtimeOwnedDclInjectedFinalOrder=window.runtimeOwnedDclInjectedOrder.join(',');});</script></body></html>";
// 300ms post-load delay (was 75ms): callers assert that the
// `WaitUntil::Load` snapshot does *not* contain the late <main id="late">
// element, while later `NetworkIdle` / `DomStable` snapshots do. Under
// nextest concurrency the load-event snapshot can take 100ms+ to land
// even though the renderer captures sync at the load event itself, so a
// 75ms gap is too tight — the setTimeout fires inside that gap and the
// negative assertion breaks. 300ms leaves a comfortable margin for both
// snapshot timing and the 5s test deadline (timer + fetch RTT << 5s).
const WAIT_UNTIL_DELAYED_FETCH_HTML: &str = "<!doctype html><html><body data-state=\"init\"><script>window.addEventListener('load', () => { setTimeout(() => { fetch('/wait-until-data').then(r => r.text()).then(text => { document.body.setAttribute('data-state', text); const main = document.createElement('main'); main.id = 'late'; main.textContent = text; document.body.appendChild(main); }); }, 300); });</script></body></html>";
const WAIT_UNTIL_COMPLETE_DELAYED_DOM_MUTATION_HTML: &str = "<!doctype html><html><body data-state=\"init\"><script>window.addEventListener('load', () => { setTimeout(() => { document.body.setAttribute('data-late-complete', 'yes'); const main = document.createElement('main'); main.id = 'late-complete'; main.textContent = 'late-complete'; document.body.appendChild(main); }, 800); });</script></body></html>";
const WAIT_UNTIL_COMPLETE_SLOW_FETCH_HTML: &str = "<!doctype html><html><body data-state=\"init\"><script>window.addEventListener('load', () => { fetch('/wait-until-very-slow-data').then(r => r.text()).then(text => { document.body.setAttribute('data-state', text); const main = document.createElement('main'); main.id = 'late-slow-fetch'; main.textContent = text; document.body.appendChild(main); }); });</script></body></html>";
const WAIT_UNTIL_COMPLETE_SLOW_XHR_HTML: &str = "<!doctype html><html><body data-state=\"init\"><script>window.addEventListener('load', () => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/wait-until-very-slow-data'); xhr.onload = () => { document.body.setAttribute('data-state', xhr.responseText); const main = document.createElement('main'); main.id = 'late-slow-xhr'; main.textContent = xhr.responseText; document.body.appendChild(main); }; xhr.send(); });</script></body></html>";
const WAIT_UNTIL_DELAYED_JSON_FETCH_HTML: &str = "<!doctype html><html><body data-state=\"init\"><script>window.addEventListener('load', () => { setTimeout(() => { fetch('/wait-until-json-data').then(r => r.json()).then(data => { document.body.setAttribute('data-state', data.ret[0]); const main = document.createElement('main'); main.id = 'late-json'; main.textContent = data.data.url; document.body.appendChild(main); }); }, 75); });</script></body></html>";
const WAIT_UNTIL_READINESS_PLAN_HTML: &str = r#"<!doctype html>
<html><head><title>readiness plan</title></head><body data-readiness-order="">
<script>
window.readinessOrder = [];
window.runtimeOwnedInOrderLoadOrder = [];
const noteReadiness = value => {
  window.readinessOrder.push(value);
  document.body.setAttribute('data-readiness-order', window.readinessOrder.join(','));
};
fetch('/wait-until-json-data').then(response => response.json()).then(data => {
  document.body.setAttribute('data-readiness-response', data.ret[0]);
  noteReadiness('response');
});
</script>
<script src="/assets/runtime_owned_in_order_load_slow.js"></script>
<script>
window.addEventListener('load', () => {
  setTimeout(() => {
    const ready = document.createElement('main');
    ready.id = 'readiness-selector';
    ready.textContent = 'selector-ready';
    document.body.appendChild(ready);
    noteReadiness('selector');
  }, 100);
  setTimeout(() => {
    window.readinessScriptReady = true;
    document.body.setAttribute('data-readiness-script', 'true');
    noteReadiness('script');
  }, 500);
});
</script>
</body></html>"#;
const WAIT_UNTIL_COOKIE_FETCH_HTML: &str = "<!doctype html><html><body data-state=\"init\"><script>window.addEventListener('load', () => { setTimeout(() => { fetch('/wait-until-cookie-data').then(r => r.json()).then(data => { document.body.setAttribute('data-state', data.cookie); const main = document.createElement('main'); main.id = 'late-cookie'; main.textContent = data.cookie; document.body.appendChild(main); }); }, 75); });</script></body></html>";
// See WAIT_UNTIL_DELAYED_FETCH_HTML for the 75 -> 300 ms rationale.
const WAIT_UNTIL_DELAYED_XHR_HTML: &str = "<!doctype html><html><body data-state=\"init\"><script>window.addEventListener('load', () => { setTimeout(() => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/wait-until-data'); xhr.onload = () => { document.body.setAttribute('data-state', xhr.responseText); const main = document.createElement('main'); main.id = 'late-xhr'; main.textContent = xhr.responseText; document.body.appendChild(main); }; xhr.send(); }, 300); });</script></body></html>";
const WAIT_UNTIL_XHR_LOCATION_REPLACE_HTML: &str = "<!doctype html><html><body data-state=\"init\"><script>window.addEventListener('load', () => { setTimeout(() => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/wait-until-data'); xhr.onload = () => { document.body.setAttribute('data-state', xhr.responseText); location.replace('/location-nav/target?from=wait-response-xhr'); }; xhr.send(); }, 75); });</script></body></html>";
const WAIT_UNTIL_STAGGERED_FETCH_HTML: &str = "<!doctype html><html><body data-first=\"init\" data-second=\"init\"><script>window.addEventListener('load', () => { setTimeout(() => { fetch('/wait-until-data').then(r => r.text()).then(text => { document.body.setAttribute('data-first', text); }); }, 75); setTimeout(() => { fetch('/wait-until-second-data').then(r => r.text()).then(text => { document.body.setAttribute('data-second', text); const main = document.createElement('main'); main.id = 'late-second'; main.textContent = text; document.body.appendChild(main); }); }, 275); });</script></body></html>";
const WAIT_UNTIL_TIMER_CALLBACK_ERROR_HTML: &str = "<!doctype html><html><body data-state=\"init\"><script>window.addEventListener('load', () => { setTimeout(() => { document.body.setAttribute('data-before-error', 'yes'); throw new Error('timer boom'); }, 0); setTimeout(() => { document.body.setAttribute('data-after-error', 'yes'); const main = document.createElement('main'); main.id = 'after-error'; main.textContent = 'after-error'; document.body.appendChild(main); }, 20); });</script></body></html>";
const WAIT_UNTIL_INTERVAL_CALLBACK_ERROR_HTML: &str = "<!doctype html><html><body data-state=\"init\"><script>window.addEventListener('load', () => { let count = 0; const id = setInterval(() => { count += 1; document.body.setAttribute('data-interval-count', String(count)); if (count === 1) { document.body.setAttribute('data-interval-before-error', 'yes'); throw new Error('interval boom'); } clearInterval(id); document.body.setAttribute('data-interval-after-error', 'yes'); const main = document.createElement('main'); main.id = 'after-interval-error'; main.textContent = 'after-interval-error'; document.body.appendChild(main); }, 20); });</script></body></html>";
const WAIT_UNTIL_TIMER_DRIVER_WRAPPER_TAMPER_HTML: &str = "<!doctype html><html><body data-state=\"init\"><script>document.body.setAttribute('data-public-timer-driver-exposed', String('__moliRunNextTimeout' in globalThis)); document.body.setAttribute('data-host-timer-driver-exposed', String('__moliHostRunNextTimeout' in globalThis)); globalThis.__moliRunNextTimeout = () => { throw new Error('tampered timer driver wrapper'); }; window.addEventListener('load', () => { setTimeout(() => { document.body.setAttribute('data-after-tamper', 'yes'); const main = document.createElement('main'); main.id = 'after-tamper'; main.textContent = 'after-tamper'; document.body.appendChild(main); }, 20); });</script></body></html>";
const WAIT_UNTIL_OUTER_HTML_TAMPER_HTML: &str = "<!doctype html><html><body data-state=\"init\"><script>Object.defineProperty(document.documentElement, 'outerHTML', { configurable: true, get() { throw new Error('domstable must not read outerHTML'); } }); window.addEventListener('load', () => { setTimeout(() => { document.body.setAttribute('data-after-outerhtml-tamper', 'yes'); const main = document.createElement('main'); main.id = 'after-outerhtml-tamper'; main.textContent = 'after-outerhtml-tamper'; document.body.appendChild(main); }, 20); });</script></body></html>";
// Keep activity frequent enough that network-idle cannot complete before the
// best-effort timeout tests reach their deadline. The fetcher's idle threshold
// is around 500 ms; a 50 ms interval leaves enough margin under nextest load.
const WAIT_UNTIL_INTERVAL_FETCH_HTML: &str = "<!doctype html><html><body data-state=\"init\"><script>window.addEventListener('load', () => { setInterval(() => { fetch('/wait-until-data').then(() => { document.body.setAttribute('data-ping', String((Number(document.body.getAttribute('data-ping') || '0') + 1))); }); }, 50); });</script></body></html>";
const WAIT_UNTIL_INTERVAL_DOM_MUTATION_HTML: &str = "<!doctype html><html><body data-state=\"init\"><main id=\"mutation-count\">0</main><script>window.addEventListener('load', () => { setInterval(() => { const count = Number(document.body.getAttribute('data-mutation-count') || '0') + 1; document.body.setAttribute('data-mutation-count', String(count)); document.getElementById('mutation-count').textContent = String(count); }, 50); });</script></body></html>";
const WAIT_UNTIL_SLOW_STATIC_HTML: &str =
    "<!doctype html><html><body><main id=\"slow-main\">slow-main=ready</main></body></html>";
const WAIT_UNTIL_HTTP_ERROR_NAVIGATION_CHALLENGE_HTML: &str = "<!doctype html><html><head><title>403 challenge</title></head><body><main id=\"challenge\">http-error-navigation=challenge</main><script>window.addEventListener('load', () => { setTimeout(() => { document.cookie = 'moli-http-error-navigation=passed; Path=/; Max-Age=3600; SameSite=Lax'; location.reload(); }, 75); });</script></body></html>";
const WAIT_UNTIL_HTTP_ERROR_LATE_NAVIGATION_CHALLENGE_HTML: &str = "<!doctype html><html><head><title>late 403 challenge</title></head><body><main id=\"challenge\">http-error-navigation=late-challenge</main><script>window.addEventListener('load', () => { setTimeout(() => { document.cookie = 'moli-http-error-navigation=passed; Path=/; Max-Age=3600; SameSite=Lax'; location.reload(); }, 1200); });</script></body></html>";
const WAIT_UNTIL_READINESS_HTTP_ERROR_CHALLENGE_HTML: &str = r#"<!doctype html>
<html><head><title>readiness 403 challenge</title></head><body>
<main id="readiness-challenge">readiness=challenge</main>
<script>
window.addEventListener('load', () => {
  setTimeout(() => {
    document.cookie = 'moli-readiness-navigation=passed; Path=/; Max-Age=3600; SameSite=Lax';
    location.reload();
  }, 250);
});
</script>
</body></html>"#;
const WAIT_UNTIL_READINESS_HTTP_ERROR_FINAL_HTML: &str = r#"<!doctype html>
<html><head><title>readiness navigation passed</title></head><body>
<main id="readiness-navigation-target">readiness=navigation-done</main>
<script>
window.addEventListener('load', () => {
  setTimeout(() => {
    fetch('/wait-until-json-data').then(response => response.json()).then(data => {
      document.body.setAttribute('data-readiness-navigation-response', data.ret[0]);
    });
  }, 100);
  setTimeout(() => {
    const ready = document.createElement('main');
    ready.id = 'readiness-navigation-selector';
    ready.textContent = 'readiness-navigation=selector';
    document.body.appendChild(ready);
  }, 300);
  setTimeout(() => {
    window.readinessNavigationScriptReady = true;
    document.body.setAttribute('data-readiness-navigation-script', 'true');
  }, 700);
});
</script>
</body></html>"#;
const WAIT_UNTIL_HTTP_ERROR_NAVIGATION_FINAL_HTML: &str = "<!doctype html><html><head><title>challenge passed</title></head><body><main id=\"http-error-navigation-target\">http-error-navigation=done</main><script>window.httpErrorNavigationDcl = false; window.httpErrorNavigationLoad = false; document.addEventListener('DOMContentLoaded', () => { window.httpErrorNavigationDcl = true; document.body.setAttribute('data-reached-dcl', 'true'); const script = document.createElement('script'); script.src = '/assets/runtime_owned_in_order_load_very_slow.js'; script.onload = () => { window.httpErrorNavigationSlowScript = true; const tail = document.createElement('main'); tail.id = 'http-error-navigation-load-tail'; tail.textContent = 'http-error-navigation=load-tail'; document.body.appendChild(tail); }; document.head.appendChild(script); }); window.addEventListener('load', () => { window.httpErrorNavigationLoad = true; document.body.setAttribute('data-reached-load', 'true'); });</script></body></html>";
const WAIT_UNTIL_HTTP_ERROR_IMMEDIATE_NAVIGATION_CHALLENGE_HTML: &str = r#"<!doctype html>
<html><head><title>immediate 403 challenge</title></head><body>
<main id="challenge">http-error-navigation=immediate-challenge</main>
<script>
window.addEventListener('load', () => {
  document.cookie = 'moli-http-error-immediate-navigation=passed; Path=/; Max-Age=3600; SameSite=Lax';
  location.reload();
});
</script>
</body></html>"#;
const WAIT_UNTIL_HTTP_ERROR_SAME_DOCUMENT_NAVIGATION_HTML: &str = r#"<!doctype html>
<html><head><title>same-document 403 challenge</title></head><body>
<main id="challenge">http-error-navigation=same-document</main>
<script>
window.addEventListener('load', () => {
  setTimeout(() => { location.hash = 'same-document-only'; }, 25);
});
</script>
</body></html>"#;
const WAIT_UNTIL_HTTP_ERROR_FIVE_NAVIGATION_CHALLENGE_HTML: &str = r#"<!doctype html>
<html><head><title>five-navigation 403 challenge</title></head><body>
<main id="challenge">http-error-navigation=five-navigation-challenge</main>
<script>
window.addEventListener('load', () => {
  setTimeout(() => {
    document.cookie = 'moli-http-error-five-navigation-step=1; Path=/; Max-Age=3600; SameSite=Lax';
    location.reload();
  }, 75);
});
</script>
</body></html>"#;
const WAIT_UNTIL_HTTP_ERROR_FIVE_NAVIGATION_FINAL_HTML: &str = r#"<!doctype html>
<html><head><title>five-navigation challenge passed</title></head><body>
<main id="http-error-five-navigation-target">http-error-navigation=five-navigation-done</main>
<script>
window.httpErrorNavigationChainStep = 5;
window.httpErrorNavigationDcl = false;
window.httpErrorNavigationLoad = false;
document.addEventListener('DOMContentLoaded', () => {
  window.httpErrorNavigationDcl = true;
  document.body.setAttribute('data-reached-dcl', 'true');
  const script = document.createElement('script');
  script.src = '/assets/runtime_owned_in_order_load_very_slow.js';
  script.onload = () => {
    window.httpErrorNavigationSlowScript = true;
    const tail = document.createElement('main');
    tail.id = 'http-error-five-navigation-load-tail';
    tail.textContent = 'http-error-navigation=five-navigation-load-tail';
    document.body.appendChild(tail);
  };
  document.head.appendChild(script);
});
window.addEventListener('load', () => {
  window.httpErrorNavigationLoad = true;
  document.body.setAttribute('data-reached-load', 'true');
});
</script>
</body></html>"#;
const WAIT_UNTIL_HTTP_ERROR_NAVIGATION_LOOP_HTML: &str = r#"<!doctype html>
<html><head><title>navigation-loop 403 challenge</title></head><body>
<main id="challenge">http-error-navigation=loop-challenge</main>
<script>
window.addEventListener('load', () => {
  setTimeout(() => { location.replace('/location-nav/loop-a'); }, 25);
});
</script>
</body></html>"#;

pub(super) fn add_wait_routes(router: Router) -> Router {
    router
        .route("/wait-until-lifecycle", get(wait_until_lifecycle_page))
        .route(
            "/wait-until-domcontentloaded-fetch",
            get(wait_until_domcontentloaded_fetch_page),
        )
        .route(
            "/wait-until-domcontentloaded-runtime-script",
            get(wait_until_domcontentloaded_runtime_script_page),
        )
        .route(
            "/wait-until-domcontentloaded-runtime-script-slow",
            get(wait_until_domcontentloaded_runtime_script_slow_page),
        )
        .route(
            "/wait-until-domcontentloaded-runtime-script-very-slow",
            get(wait_until_domcontentloaded_runtime_script_very_slow_page),
        )
        .route(
            "/wait-until-delayed-fetch",
            get(wait_until_delayed_fetch_page),
        )
        .route(
            "/wait-until-complete-delayed-dom-mutation",
            get(wait_until_complete_delayed_dom_mutation_page),
        )
        .route(
            "/wait-until-complete-slow-fetch",
            get(wait_until_complete_slow_fetch_page),
        )
        .route(
            "/wait-until-complete-slow-xhr",
            get(wait_until_complete_slow_xhr_page),
        )
        .route(
            "/wait-until-delayed-json-fetch",
            get(wait_until_delayed_json_fetch_page),
        )
        .route(
            "/wait-until-readiness-plan",
            get(wait_until_readiness_plan_page),
        )
        .route(
            "/wait-until-cookie-fetch",
            get(wait_until_cookie_fetch_page),
        )
        .route("/wait-until-delayed-xhr", get(wait_until_delayed_xhr_page))
        .route(
            "/wait-until-xhr-location-replace",
            get(wait_until_xhr_location_replace_page),
        )
        .route(
            "/wait-until-staggered-fetch",
            get(wait_until_staggered_fetch_page),
        )
        .route(
            "/wait-until-timer-callback-error",
            get(wait_until_timer_callback_error_page),
        )
        .route(
            "/wait-until-interval-callback-error",
            get(wait_until_interval_callback_error_page),
        )
        .route(
            "/wait-until-timer-driver-wrapper-tamper",
            get(wait_until_timer_driver_wrapper_tamper_page),
        )
        .route(
            "/wait-until-outer-html-tamper",
            get(wait_until_outer_html_tamper_page),
        )
        .route(
            "/wait-until-interval-fetch",
            get(wait_until_interval_fetch_page),
        )
        .route(
            "/wait-until-interval-dom-mutation",
            get(wait_until_interval_dom_mutation_page),
        )
        .route("/wait-until-slow-static", get(wait_until_slow_static_page))
        .route(
            "/wait-until-slow-interval-fetch",
            get(wait_until_slow_interval_fetch_page),
        )
        .route(
            "/wait-until-slow-interval-dom-mutation",
            get(wait_until_slow_interval_dom_mutation_page),
        )
        .route(
            "/wait-until-http-error-navigation",
            get(wait_until_http_error_navigation_page),
        )
        .route(
            "/wait-until-http-error-navigation-to-error",
            get(wait_until_http_error_navigation_to_error_page),
        )
        .route(
            "/wait-until-http-error-immediate-navigation",
            get(wait_until_http_error_immediate_navigation_page),
        )
        .route(
            "/wait-until-http-error-same-document-navigation",
            get(wait_until_http_error_same_document_navigation_page),
        )
        .route(
            "/wait-until-http-error-five-navigations",
            get(wait_until_http_error_five_navigations_page),
        )
        .route(
            "/wait-until-http-error-navigation-loop",
            get(wait_until_http_error_navigation_loop_page),
        )
        .route(
            "/wait-until-http-error-late-navigation",
            get(wait_until_http_error_late_navigation_page),
        )
        .route(
            "/wait-until-readiness-http-error-navigation",
            get(wait_until_readiness_http_error_navigation_page),
        )
        .route("/wait-until-data", get(wait_until_data_page))
        .route("/wait-until-json-data", get(wait_until_json_data_page))
        .route("/wait-until-cookie-data", get(wait_until_cookie_data_page))
        .route("/wait-until-slow-data", get(wait_until_slow_data_page))
        .route(
            "/wait-until-very-slow-data",
            get(wait_until_very_slow_data_page),
        )
        .route("/wait-until-second-data", get(wait_until_second_data_page))
}

async fn wait_until_lifecycle_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_LIFECYCLE_HTML)
}

async fn wait_until_http_error_navigation_page(headers: HeaderMap) -> Response {
    if has_cookie(&headers, "moli-http-error-navigation=passed") {
        return Html(WAIT_UNTIL_HTTP_ERROR_NAVIGATION_FINAL_HTML).into_response();
    }

    (
        StatusCode::FORBIDDEN,
        Html(WAIT_UNTIL_HTTP_ERROR_NAVIGATION_CHALLENGE_HTML),
    )
        .into_response()
}

async fn wait_until_http_error_navigation_to_error_page(headers: HeaderMap) -> Response {
    if has_cookie(&headers, "moli-http-error-navigation=passed") {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(WAIT_UNTIL_HTTP_ERROR_NAVIGATION_FINAL_HTML),
        )
            .into_response();
    }

    (
        StatusCode::FORBIDDEN,
        Html(WAIT_UNTIL_HTTP_ERROR_NAVIGATION_CHALLENGE_HTML),
    )
        .into_response()
}

async fn wait_until_http_error_immediate_navigation_page(headers: HeaderMap) -> Response {
    if has_cookie(&headers, "moli-http-error-immediate-navigation=passed") {
        return Html(WAIT_UNTIL_HTTP_ERROR_NAVIGATION_FINAL_HTML).into_response();
    }

    (
        StatusCode::FORBIDDEN,
        Html(WAIT_UNTIL_HTTP_ERROR_IMMEDIATE_NAVIGATION_CHALLENGE_HTML),
    )
        .into_response()
}

async fn wait_until_http_error_same_document_navigation_page() -> Response {
    (
        StatusCode::FORBIDDEN,
        Html(WAIT_UNTIL_HTTP_ERROR_SAME_DOCUMENT_NAVIGATION_HTML),
    )
        .into_response()
}

async fn wait_until_http_error_five_navigations_page(headers: HeaderMap) -> Response {
    let step = (1_u8..=5)
        .find(|step| {
            has_cookie(
                &headers,
                &format!("moli-http-error-five-navigation-step={step}"),
            )
        })
        .unwrap_or(0);
    if step == 0 {
        return (
            StatusCode::FORBIDDEN,
            Html(WAIT_UNTIL_HTTP_ERROR_FIVE_NAVIGATION_CHALLENGE_HTML),
        )
            .into_response();
    }
    if step == 5 {
        return Html(WAIT_UNTIL_HTTP_ERROR_FIVE_NAVIGATION_FINAL_HTML).into_response();
    }

    let next_step = step + 1;
    Html(format!(
        r#"<!doctype html>
<html><head><title>five-navigation step {step}</title></head><body>
<main id="http-error-five-navigation-step-{step}">step={step}</main>
<script>
document.cookie = 'moli-http-error-five-navigation-step={next_step}; Path=/; Max-Age=3600; SameSite=Lax';
location.reload();
</script>
</body></html>"#
    ))
    .into_response()
}

async fn wait_until_http_error_navigation_loop_page() -> Response {
    (
        StatusCode::FORBIDDEN,
        Html(WAIT_UNTIL_HTTP_ERROR_NAVIGATION_LOOP_HTML),
    )
        .into_response()
}

async fn wait_until_http_error_late_navigation_page(headers: HeaderMap) -> Response {
    if has_cookie(&headers, "moli-http-error-navigation=passed") {
        return Html(WAIT_UNTIL_HTTP_ERROR_NAVIGATION_FINAL_HTML).into_response();
    }

    (
        StatusCode::FORBIDDEN,
        Html(WAIT_UNTIL_HTTP_ERROR_LATE_NAVIGATION_CHALLENGE_HTML),
    )
        .into_response()
}

async fn wait_until_readiness_http_error_navigation_page(headers: HeaderMap) -> Response {
    if has_cookie(&headers, "moli-readiness-navigation=passed") {
        return Html(WAIT_UNTIL_READINESS_HTTP_ERROR_FINAL_HTML).into_response();
    }

    (
        StatusCode::FORBIDDEN,
        Html(WAIT_UNTIL_READINESS_HTTP_ERROR_CHALLENGE_HTML),
    )
        .into_response()
}

async fn wait_until_domcontentloaded_fetch_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_DOMCONTENTLOADED_FETCH_HTML)
}

async fn wait_until_domcontentloaded_runtime_script_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_DOMCONTENTLOADED_RUNTIME_SCRIPT_HTML)
}

async fn wait_until_domcontentloaded_runtime_script_slow_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_DOMCONTENTLOADED_RUNTIME_SCRIPT_SLOW_HTML)
}

async fn wait_until_domcontentloaded_runtime_script_very_slow_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_DOMCONTENTLOADED_RUNTIME_SCRIPT_VERY_SLOW_HTML)
}

async fn wait_until_delayed_fetch_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_DELAYED_FETCH_HTML)
}

async fn wait_until_complete_delayed_dom_mutation_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_COMPLETE_DELAYED_DOM_MUTATION_HTML)
}

async fn wait_until_complete_slow_fetch_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_COMPLETE_SLOW_FETCH_HTML)
}

async fn wait_until_complete_slow_xhr_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_COMPLETE_SLOW_XHR_HTML)
}

async fn wait_until_delayed_json_fetch_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_DELAYED_JSON_FETCH_HTML)
}

async fn wait_until_readiness_plan_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_READINESS_PLAN_HTML)
}

async fn wait_until_cookie_fetch_page() -> Response {
    (
        [(
            SET_COOKIE,
            HeaderValue::from_static("trace_login=fixture; Path=/; SameSite=Lax"),
        )],
        Html(WAIT_UNTIL_COOKIE_FETCH_HTML),
    )
        .into_response()
}

async fn wait_until_delayed_xhr_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_DELAYED_XHR_HTML)
}

async fn wait_until_xhr_location_replace_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_XHR_LOCATION_REPLACE_HTML)
}

async fn wait_until_staggered_fetch_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_STAGGERED_FETCH_HTML)
}

async fn wait_until_timer_callback_error_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_TIMER_CALLBACK_ERROR_HTML)
}

async fn wait_until_interval_callback_error_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_INTERVAL_CALLBACK_ERROR_HTML)
}

async fn wait_until_timer_driver_wrapper_tamper_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_TIMER_DRIVER_WRAPPER_TAMPER_HTML)
}

async fn wait_until_outer_html_tamper_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_OUTER_HTML_TAMPER_HTML)
}

async fn wait_until_interval_fetch_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_INTERVAL_FETCH_HTML)
}

async fn wait_until_interval_dom_mutation_page() -> Html<&'static str> {
    Html(WAIT_UNTIL_INTERVAL_DOM_MUTATION_HTML)
}

// These delayed main-resource fixtures make timeout accounting observable:
// the initial 500 ms must be deducted before NetworkIdle/DomStable starts.
// A fresh readiness timeout would incorrectly extend the total by 500 ms.
async fn wait_until_slow_static_page() -> Html<&'static str> {
    sleep(Duration::from_millis(500)).await;
    Html(WAIT_UNTIL_SLOW_STATIC_HTML)
}

async fn wait_until_slow_interval_fetch_page() -> Html<&'static str> {
    sleep(Duration::from_millis(500)).await;
    Html(WAIT_UNTIL_INTERVAL_FETCH_HTML)
}

async fn wait_until_slow_interval_dom_mutation_page() -> Html<&'static str> {
    sleep(Duration::from_millis(500)).await;
    Html(WAIT_UNTIL_INTERVAL_DOM_MUTATION_HTML)
}

async fn wait_until_data_page() -> Response {
    (
        [(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )],
        "settled",
    )
        .into_response()
}

async fn wait_until_json_data_page() -> Response {
    (
        [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
        r#"{"api":"fixture.detail","ret":["SUCCESS"],"data":{"url":"/item/42"}}"#,
    )
        .into_response()
}

async fn wait_until_cookie_data_page(headers: HeaderMap) -> Response {
    let cookie = if has_cookie(&headers, "trace_login=fixture") {
        "present"
    } else {
        "missing"
    };
    (
        [
            (CONTENT_TYPE, HeaderValue::from_static("application/json")),
            (
                SET_COOKIE,
                HeaderValue::from_static("trace_seen=1; Path=/; SameSite=Lax"),
            ),
        ],
        format!(r#"{{"api":"fixture.cookie","cookie":"{cookie}"}}"#),
    )
        .into_response()
}

async fn wait_until_slow_data_page() -> Response {
    sleep(Duration::from_millis(300)).await;
    (
        [(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )],
        "settled-slow",
    )
        .into_response()
}

async fn wait_until_very_slow_data_page() -> Response {
    sleep(Duration::from_millis(1500)).await;
    (
        [(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )],
        "settled-very-slow",
    )
        .into_response()
}

async fn wait_until_second_data_page() -> Response {
    (
        [(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )],
        "settled-second",
    )
        .into_response()
}
