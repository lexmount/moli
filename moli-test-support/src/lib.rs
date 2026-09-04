use anyhow::Context;
use anyhow::Result;
use axum::{
    Extension, Router,
    body::{Body, Bytes},
    extract::{Query, Request as AxumRequest},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CONTENT_TYPE, COOKIE, SET_COOKIE},
    },
    response::{Html, IntoResponse, Redirect, Response},
    routing::{any, get},
};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::{
    net::TcpListener,
    sync::{Notify, oneshot},
    task::JoinHandle,
    time::{Duration, sleep},
};

#[derive(Clone, Default)]
struct FixtureRuntimeState {
    dynamic_stylesheet_dcl: Arc<FixtureEvent>,
    dynamic_stylesheet_script_executed: Arc<FixtureEvent>,
}

#[derive(Default)]
struct FixtureEvent {
    signaled: AtomicBool,
    notify: Notify,
}

impl FixtureEvent {
    fn signal(&self) {
        if !self.signaled.swap(true, Ordering::Release) {
            self.notify.notify_waiters();
        }
    }

    async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            if self.signaled.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

mod helpers;
mod routes_core;
mod routes_wait;
mod server;
mod server_routes;

use helpers::*;
use routes_core::*;
const STATIC_HTML: &str = "<!doctype html><html><body><main>fixture static</main></body></html>";
const FUTURE_INTERVAL_DONE_HTML: &str = "<!doctype html><html><body><main>interval</main><script>document.body.dataset.ready = 'yes'; setInterval(() => { document.body.dataset.ticks = String((Number(document.body.dataset.ticks) || 0) + 1); }, 50);</script></body></html>";
const SCRIPT_HTML: &str =
    "<!doctype html><html><body><script src=\"/assets/app.js\"></script></body></html>";
const INLINE_SCRIPT_HTML: &str = "<!doctype html><html><head><!--head--><script>window.inlineReady = '\u{4f60}\u{597d}';</script></head><body><template id=\"tpl\"><script>window.templateReady = true;</script></template><main>fixture inline script</main></body></html>";
const SCRIPT_EXECUTION_HTML: &str = "<!doctype html><html><head><script>window.executionOrder = ['inline-normal']; window.inlineReady = '\u{4f60}\u{597d}';</script><script src=\"/assets/sequence.js\"></script><script defer>window.executionOrder.push('inline-defer'); window.deferReady = true;</script><script async>window.executionOrder.push('inline-async'); window.executionOrderText = window.executionOrder.join(','); window.asyncReady = true;</script><script type=\"application/json\">{\"ignored\":true}</script><script type=\"module\">window.moduleReady = true;</script><script type=\"importmap\">{\"imports\":{\"fixture\":\"/assets/sequence.js\"}}</script></head><body><template id=\"tpl\"><script>window.templateShouldNotRun = true;</script></template><main>fixture script execution</main></body></html>";
const COOKIE_MISSING_HTML: &str =
    "<!doctype html><html><body><main>cookie=missing</main></body></html>";
const COOKIE_SEEN_HTML: &str = "<!doctype html><html><body><main>cookie=seen</main></body></html>";
const COOKIE_SCOPE_MISSING_HTML: &str =
    "<!doctype html><html><body><main>scoped-cookie=missing</main></body></html>";
const COOKIE_SCOPE_SEEN_HTML: &str =
    "<!doctype html><html><body><main>scoped-cookie=seen</main></body></html>";
const COOKIE_DOMAIN_MISSING_HTML: &str =
    "<!doctype html><html><body><main>invalid-domain-cookie=missing</main></body></html>";
const COOKIE_DOMAIN_SEEN_HTML: &str =
    "<!doctype html><html><body><main>invalid-domain-cookie=seen</main></body></html>";
const COOKIE_REPLACE_RED_HTML: &str =
    "<!doctype html><html><body><main>replace-cookie=red</main></body></html>";
const COOKIE_REPLACE_BLUE_HTML: &str =
    "<!doctype html><html><body><main>replace-cookie=blue</main></body></html>";
const COOKIE_CHAIN_OK_HTML: &str = "<!doctype html><html><head><title>Cookie chain final</title></head><body><main>cookie-chain=ok</main></body></html>";
const COOKIE_CHAIN_BROKEN_HTML: &str =
    "<!doctype html><html><body><main>cookie-chain=broken</main></body></html>";
const COOKIE_LOCATION_GATE_MISSING_HTML: &str = "<!doctype html><html><body><main>cookie-location-gate=missing</main><script>document.cookie='ttwid=fixture; Path=/; Max-Age=3600; SameSite=Lax'; if (!location.search.includes('wid=')) location.replace(location.pathname + '?wid=fixture');</script></body></html>";
const COOKIE_LOCATION_GATE_SEEN_HTML: &str =
    "<!doctype html><html><body><main>cookie-location-gate=seen</main></body></html>";
const SLOW_A_HTML: &str = "<!doctype html><html><body><main>slow=a</main></body></html>";
const SLOW_B_HTML: &str = "<!doctype html><html><body><main>slow=b</main></body></html>";
const LOCATION_NAV_REPLACE_SOURCE_HTML: &str = "<!doctype html><html><body><main id=\"source\">replace-source</main><script>location.replace('/location-nav/target?from=replace'); window.locationReplaceAfterCall = location.href;</script></body></html>";
const LOCATION_NAV_ASSIGN_SOURCE_HTML: &str = "<!doctype html><html><body><main id=\"source\">assign-source</main><script>location.assign('/location-nav/target?from=assign'); window.locationAssignAfterCall = location.href;</script></body></html>";
const LOCATION_NAV_HREF_SOURCE_HTML: &str = "<!doctype html><html><body><main id=\"source\">href-source</main><script>location.href = '/location-nav/target?from=href'; window.locationHrefAfterCall = location.href;</script></body></html>";
const LOCATION_NAV_PATHNAME_SOURCE_HTML: &str = "<!doctype html><html><body><main id=\"source\">pathname-source</main><script>location.pathname = '/location-nav/pathname-target'; window.locationPathnameAfterCall = location.href;</script></body></html>";
const LOCATION_NAV_SEARCH_SOURCE_HTML: &str = "<!doctype html><html><body><main id=\"source\">search-source</main><script>location.search = 'from=search'; window.locationSearchAfterCall = location.href;</script></body></html>";
const LOCATION_NAV_SEARCH_ASYNC_SOURCE_HTML: &str = "<!doctype html><html><body><main id=\"source\">search-async-source</main><script>setTimeout(() => { location.search = 'from=search-async'; }, 10);</script></body></html>";
const LOCATION_NAV_ASSIGN_POST_PARSE_TIMEOUT_SOURCE_HTML: &str = "<!doctype html><html><body><main id=\"source\">assign-post-parse-timeout-source</main><script>setTimeout(() => { location.assign('/location-nav/target?from=assign-post-parse-timeout'); }, 0);</script></body></html>";
const LOCATION_NAV_RELOAD_SOURCE_HTML: &str = "<!doctype html><html><body><main id=\"source\">reload-source</main><script>location.reload(); window.locationReloadAfterCall = location.href;</script></body></html>";
const LOCATION_NAV_POST_LOAD_COOKIE_RELOAD_CHALLENGE_HTML: &str = "<!doctype html><html><head><title>post-load challenge</title></head><body><main id=\"source\">post-load-cookie-reload=source</main><script>window.addEventListener('load', () => { setTimeout(() => { document.cookie = 'lm-post-load-cookie-reload=1; Path=/location-nav; Max-Age=3600; SameSite=Lax'; location.reload(); }, 50); });</script></body></html>";
const LOCATION_NAV_POST_LOAD_COOKIE_RELOAD_FINAL_HTML: &str = "<!doctype html><html><head><title>post-load challenge passed</title><script src=\"/location-nav/post-load-cookie-reload-final.js\"></script></head><body><main id=\"target\">post-load-cookie-reload=done</main></body></html>";
const LOCATION_NAV_SAME_HREF_COOKIE_CHALLENGE_HTML: &str = "<!doctype html><html><head><title>challenge</title><script src=\"/location-nav/same-href-cookie-challenge-sdk.js\"></script></head><body><main id=\"source\">same-href-cookie-challenge=source</main><script>if (window.EOJsChallengeSDK) { new window.EOJsChallengeSDK({ callback: function(token) { document.cookie = 'EO-Bot-Js-Token=' + token + '; Path=/location-nav; Max-Age=3600; SameSite=Lax'; location.href = location.href.replace(/[?&]tads/, ''); } }).start(); } else { document.body.setAttribute('data-missing-sdk', 'true'); }</script></body></html>";
const LOCATION_NAV_SAME_HREF_COOKIE_CHALLENGE_FINAL_HTML: &str = "<!doctype html><html><body><main id=\"target\">same-href-cookie-challenge=done</main></body></html>";
const LOCATION_NAV_SAME_HREF_COOKIE_CHALLENGE_SDK: &str = "window.EOJsChallengeSDK = function(options) { this.start = function() { options.callback('test-token'); }; };";
const LOCATION_NAV_CHAIN_SOURCE_HTML: &str = "<!doctype html><html><body><main id=\"source\">chain-source</main><script>location.assign('/location-nav/chain-mid');</script></body></html>";
const LOCATION_NAV_CHAIN_MID_HTML: &str = "<!doctype html><html><body><main id=\"mid\">chain-mid</main><script>location.replace('/location-nav/target?from=chain-mid');</script></body></html>";
const LOCATION_NAV_CHAIN_TIMEOUT_SOURCE_HTML: &str = "<!doctype html><html><body><main id=\"chain-timeout-source\">chain-timeout-source</main><script>setTimeout(() => { location.assign('/location-nav/chain-source'); }, 0);</script></body></html>";
const LOCATION_NAV_LOOP_TIMEOUT_SOURCE_HTML: &str = "<!doctype html><html><body><main id=\"loop-timeout-source\">loop-timeout-source</main><script>setTimeout(() => { location.assign('/location-nav/loop-a'); }, 0);</script></body></html>";
const LOCATION_NAV_LOOP_A_HTML: &str = "<!doctype html><html><body><main id=\"loop-a\">loop-a</main><script>location.replace('/location-nav/loop-b');</script></body></html>";
const LOCATION_NAV_LOOP_B_HTML: &str = "<!doctype html><html><body><main id=\"loop-b\">loop-b</main><script>location.replace('/location-nav/loop-a');</script></body></html>";
const DATE_LOCALE_BOMB_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>bomb</title><script>for(let i=0;i<2000;i++){new Date(1711267200000+i*1000).toLocaleString('en-US');new Date(1711267200000+i*1000).toLocaleDateString('en-US');new Date(1711267200000+i*1000).toLocaleTimeString('en-US');}document.documentElement.setAttribute('data-ok','1');</script></head><body>ok</body></html>";
const BROWSER_SURFACE_COMPAT_HTML: &str = concat!(
    "<!doctype html><html><body data-mime-length=\"\" data-plugin-length=\"\" ",
    "data-pdf-viewer-enabled=\"\" data-storage-type=\"\" data-storage-instance=\"\" ",
    "data-window-add-event-listener=\"\" data-window-remove-event-listener=\"\" ",
    "data-window-dispatch-event=\"\" data-global-add-event-listener=\"\" ",
    "data-global-remove-event-listener=\"\" data-global-dispatch-event=\"\" ",
    "data-history-length-after-push=\"\" data-history-state-after-push=\"\" ",
    "data-history-state-after-replace=\"\" data-location-after-push=\"\" ",
    "data-location-after-replace=\"\"><script>",
    "document.body.setAttribute('data-mime-length', String(navigator.mimeTypes.length));",
    "document.body.setAttribute('data-plugin-length', String(navigator.plugins.length));",
    "document.body.setAttribute('data-pdf-viewer-enabled', String(navigator.pdfViewerEnabled));",
    "document.body.setAttribute('data-storage-type', typeof Storage);",
    "document.body.setAttribute('data-storage-instance', String(localStorage instanceof Storage && sessionStorage instanceof Storage));",
    "document.body.setAttribute('data-window-add-event-listener', typeof window.addEventListener);",
    "document.body.setAttribute('data-window-remove-event-listener', typeof window.removeEventListener);",
    "document.body.setAttribute('data-window-dispatch-event', typeof window.dispatchEvent);",
    "document.body.setAttribute('data-global-add-event-listener', typeof addEventListener);",
    "document.body.setAttribute('data-global-remove-event-listener', typeof removeEventListener);",
    "document.body.setAttribute('data-global-dispatch-event', typeof dispatchEvent);",
    "history.pushState({step:1}, '', '/compat/history-pushed?from=push');",
    "document.body.setAttribute('data-history-length-after-push', String(history.length));",
    "document.body.setAttribute('data-history-state-after-push', JSON.stringify(history.state));",
    "document.body.setAttribute('data-location-after-push', location.pathname + location.search);",
    "history.replaceState({step:2}, '', '/compat/history-replaced?from=replace');",
    "document.body.setAttribute('data-history-state-after-replace', JSON.stringify(history.state));",
    "document.body.setAttribute('data-location-after-replace', location.pathname + location.search);",
    "</script></body></html>"
);
const DATE_LOCALE_DETAILS_HTML: &str = "<!doctype html><html><body data-locale-string=\"\" data-locale-date=\"\" data-locale-time=\"\" data-invalid=\"\"><script>const d=new Date(Date.UTC(2024,2,24,16,5,6));document.body.setAttribute('data-locale-string', d.toLocaleString('en-US'));document.body.setAttribute('data-locale-date', d.toLocaleDateString('en-US'));document.body.setAttribute('data-locale-time', d.toLocaleTimeString('en-US'));document.body.setAttribute('data-invalid', new Date(NaN).toLocaleString('en-US'));</script></body></html>";
const BROWSER_SURFACE_DETAILS_HTML: &str = "<!doctype html><html><body data-mime-tag=\"\" data-plugin-tag=\"\" data-storage-tag=\"\" data-mime-item-hit=\"\" data-mime-named-item-hit=\"\" data-plugin-item-hit=\"\" data-plugin-named-item-hit=\"\" data-mime-item-null=\"\" data-mime-named-item-null=\"\" data-plugin-item-null=\"\" data-plugin-named-item-null=\"\" data-plugin-refresh-undefined=\"\" data-storage-prototype=\"\" data-storage-roundtrip=\"\" data-storage-length-after-set=\"\" data-storage-key0=\"\" data-storage-length-after-remove=\"\" data-history-scroll-restoration=\"\"><script>document.body.setAttribute('data-mime-tag', Object.prototype.toString.call(navigator.mimeTypes));document.body.setAttribute('data-plugin-tag', Object.prototype.toString.call(navigator.plugins));document.body.setAttribute('data-storage-tag', Object.prototype.toString.call(localStorage));document.body.setAttribute('data-mime-item-hit', String(navigator.mimeTypes.item(0)?.type ?? 'null'));document.body.setAttribute('data-mime-named-item-hit', String(navigator.mimeTypes.namedItem('application/pdf')?.type ?? 'null'));document.body.setAttribute('data-plugin-item-hit', String(navigator.plugins.item(0)?.name ?? 'null'));document.body.setAttribute('data-plugin-named-item-hit', String(navigator.plugins.namedItem('PDF Viewer')?.name ?? 'null'));document.body.setAttribute('data-mime-item-null', String(navigator.mimeTypes.item(999)===null));document.body.setAttribute('data-mime-named-item-null', String(navigator.mimeTypes.namedItem('application/x-missing')===null));document.body.setAttribute('data-plugin-item-null', String(navigator.plugins.item(999)===null));document.body.setAttribute('data-plugin-named-item-null', String(navigator.plugins.namedItem('Missing Plugin')===null));document.body.setAttribute('data-plugin-refresh-undefined', String(navigator.plugins.refresh()===undefined));document.body.setAttribute('data-storage-prototype', String(Storage.prototype.isPrototypeOf(localStorage) && Storage.prototype.isPrototypeOf(sessionStorage)));localStorage.clear();localStorage.setItem('alpha','1');document.body.setAttribute('data-storage-roundtrip', String(localStorage.getItem('alpha')));document.body.setAttribute('data-storage-length-after-set', String(localStorage.length));document.body.setAttribute('data-storage-key0', String(localStorage.key(0)));localStorage.removeItem('alpha');document.body.setAttribute('data-storage-length-after-remove', String(localStorage.length));document.body.setAttribute('data-history-scroll-restoration', String(history.scrollRestoration));</script></body></html>";
const HISTORY_RELATIVE_URL_UPDATE_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/history_relative_url_update.html");
const HISTORY_PUSHSTATE_DOES_NOT_SET_NAVIGATION_CURRENT_ENTRY_STATE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_pushstate_does_not_set_navigation_current_entry_state.html"
);
const HISTORY_STATE_CLONE_AND_DATACLONE_ERROR_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_state_clone_and_dataclone_error.html"
);
const HISTORY_CROSS_ORIGIN_SECURITY_ERROR_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/history_cross_origin_security_error.html");
const HISTORY_GO_ZERO_RELOADS_CURRENT_DOCUMENT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_go_zero_reloads_current_document.html"
);
const HISTORY_GO_NAN_RELOADS_CURRENT_DOCUMENT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_go_nan_reloads_current_document.html"
);
const HISTORY_GO_NO_ARGUMENT_RELOADS_CURRENT_DOCUMENT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_go_no_argument_reloads_current_document.html"
);
const HISTORY_GO_REJECTS_SYMBOL_AND_BIGINT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_go_rejects_symbol_and_bigint.html"
);
const HISTORY_GO_STRING_MINUS_ONE_TRAVERSES_BACK_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_go_string_minus_one_traverses_back.html"
);
const HISTORY_BACK_SAME_TURN_TRAVERSES_ASYNCHRONOUSLY_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_back_same_turn_traverses_asynchronously.html"
);
const HISTORY_BACK_IGNORES_PAGE_TAMPERED_QUEUE_MICROTASK_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_back_ignores_page_tampered_queue_microtask.html"
);
const HISTORY_BACK_FORWARD_SAME_TURN_COALESCES_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_back_forward_same_turn_coalesces.html"
);
const HISTORY_STATE_MUTATION_DOES_NOT_MUTATE_STORED_SNAPSHOT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_state_mutation_does_not_mutate_stored_snapshot.html"
);
const HISTORY_LENGTH_AND_STATE_ASSIGNMENTS_DO_NOT_MUTATE_PUBLIC_SURFACE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_length_and_state_assignments_do_not_mutate_public_surface.html"
);
const HISTORY_NAVIGATION_BRAND_AND_DESCRIPTOR_SURFACE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_navigation_brand_and_descriptor_surface.html"
);
const HISTORY_SCROLL_RESTORATION_INVALID_VALUE_IGNORED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_scroll_restoration_invalid_value_ignored.html"
);
const HISTORY_LOCATION_HASH_ASSIGNMENT_DISPATCHES_POPSTATE_AND_HASHCHANGE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_location_hash_assignment_dispatches_popstate_and_hashchange.html"
);
const NAVIGATION_CURRENTENTRYCHANGE_ON_HASH_NAVIGATION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_currententrychange_on_hash_navigation.html"
);
const NAVIGATION_CURRENTENTRYCHANGE_IGNORES_PAGE_TAMPERED_DISPATCH_EVENT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_currententrychange_ignores_page_tampered_dispatch_event.html"
);
const NAVIGATION_UPDATE_CURRENT_ENTRY_UPDATES_STATE_AND_FIRES_CURRENTENTRYCHANGE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_update_current_entry_updates_state_and_fires_currententrychange.html"
);
const HISTORY_PUSHSTATE_DISPATCHES_NAVIGATION_CURRENTENTRYCHANGE_EVENT_SURFACE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_pushstate_dispatches_navigation_currententrychange_event_surface.html"
);
const NAVIGATION_RELOAD_RELOADS_CURRENT_DOCUMENT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_reload_reloads_current_document.html"
);
const NAVIGATION_BACK_SURFACE_AND_FRAGMENT_TRAVERSAL_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_back_surface_and_fragment_traversal.html"
);
const NAVIGATION_TRAVERSE_TO_KEY_FRAGMENT_TRAVERSAL_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_traverse_to_key_fragment_traversal.html"
);
const NAVIGATION_ONCURRENTENTRYCHANGE_PROPERTY_RECEIVES_TRAVERSE_EVENT_SURFACE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_oncurrententrychange_property_receives_traverse_event_surface.html"
);
const NAVIGATION_FORWARD_DISPATCHES_CURRENTENTRYCHANGE_TRAVERSE_EVENT_SURFACE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_forward_dispatches_currententrychange_traverse_event_surface.html"
);
const NAVIGATION_FORWARD_RESULT_PROMISES_SETTLE_AFTER_ASYNC_TRAVERSAL_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_forward_result_promises_settle_after_async_traversal.html"
);
const NAVIGATION_BACK_RESULT_PROMISES_SETTLE_AFTER_ASYNC_TRAVERSAL_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_back_result_promises_settle_after_async_traversal.html"
);
const NAVIGATION_TRAVERSE_TO_RESULT_PROMISES_SETTLE_AFTER_ASYNC_TRAVERSAL_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_traverse_to_result_promises_settle_after_async_traversal.html"
);
const NAVIGATION_BACK_RESTORES_NAVIGATION_ENTRY_STATE_SEPARATELY_FROM_HISTORY_STATE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_back_restores_navigation_entry_state_separately_from_history_state.html"
);
const NAVIGATION_TRAVERSE_TO_RESTORES_NAVIGATION_ENTRY_STATE_SEPARATELY_FROM_HISTORY_STATE_HTML:
    &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_traverse_to_restores_navigation_entry_state_separately_from_history_state.html"
);
const NAVIGATION_NAVIGATE_STATE_PERSISTS_TO_DESTINATION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_navigate_state_persists_to_destination.html"
);
const NAVIGATION_NAVIGATE_STATE_DESTINATION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_navigate_state_destination.html"
);
const NAVIGATION_NAVIGATE_CROSS_DOCUMENT_RESULT_PROMISES_DO_NOT_SETTLE_BEFORE_DESTINATION_LOAD_HTML:
    &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_navigate_cross_document_result_promises_do_not_settle_before_destination_load.html"
);
const NAVIGATION_NAVIGATE_RESULT_PROMISES_DESTINATION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_navigate_result_promises_destination.html"
);
const NAVIGATION_NAVIGATE_SAME_DOCUMENT_PUSH_UPDATES_HISTORY_AND_EVENTS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_navigate_same_document_push_updates_history_and_events.html"
);
const NAVIGATION_NAVIGATE_SAME_DOCUMENT_REPLACE_UPDATES_HISTORY_AND_EVENTS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_navigate_same_document_replace_updates_history_and_events.html"
);
const NAVIGATION_NAVIGATE_ARGUMENT_VALIDATION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_navigate_argument_validation.html"
);
const NAVIGATION_NAVIGATE_SAME_DOCUMENT_RESULT_PROMISES_SETTLE_BEFORE_HASHCHANGE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_navigate_same_document_result_promises_settle_before_hashchange.html"
);
const NAVIGATION_NAVIGATE_SAME_DOCUMENT_STATE_USES_STRUCTURED_CLONE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_navigate_same_document_state_uses_structured_clone.html"
);
const NAVIGATION_NAVIGATE_CROSS_DOCUMENT_DOES_NOT_DISPATCH_CURRENTENTRYCHANGE_IN_SOURCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_navigate_cross_document_does_not_dispatch_currententrychange_in_source.html"
);
const NAVIGATION_NAVIGATE_CROSS_DOCUMENT_DOES_NOT_DISPATCH_CURRENTENTRYCHANGE_DESTINATION_HTML:
    &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_navigate_cross_document_does_not_dispatch_currententrychange_destination.html"
);
const NAVIGATION_ACTIVATION_INITIAL_SURFACE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_activation_initial_surface.html"
);
const NAVIGATION_ACTIVATION_SAME_DOCUMENT_NAVIGATION_STAYS_INITIAL_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_activation_same_document_navigation_stays_initial.html"
);
const NAVIGATION_ACTIVATION_CROSS_DOCUMENT_DESTINATION_SURFACE_SOURCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_activation_cross_document_destination_surface_source.html"
);
const NAVIGATION_ACTIVATION_CROSS_DOCUMENT_DESTINATION_SURFACE_DEST_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_activation_cross_document_destination_surface_dest.html"
);
const NAVIGATION_ACTIVATION_CROSS_DOCUMENT_BACK_DESTINATION_SURFACE_SOURCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_activation_cross_document_back_destination_surface_source.html"
);
const NAVIGATION_ACTIVATION_CROSS_DOCUMENT_BACK_DESTINATION_SURFACE_DEST_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_activation_cross_document_back_destination_surface_dest.html"
);
const NAVIGATION_ACTIVATION_CROSS_DOCUMENT_TRAVERSE_TO_DESTINATION_SURFACE_SOURCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_activation_cross_document_traverse_to_destination_surface_source.html"
);
const NAVIGATION_ACTIVATION_CROSS_DOCUMENT_TRAVERSE_TO_DESTINATION_SURFACE_DEST_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_activation_cross_document_traverse_to_destination_surface_dest.html"
);
const NAVIGATION_NAVIGATE_CROSS_DOCUMENT_PUSH_DESTINATION_SURFACE_SOURCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_navigate_cross_document_push_destination_surface_source.html"
);
const NAVIGATION_NAVIGATE_CROSS_DOCUMENT_PUSH_DESTINATION_SURFACE_DEST_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_navigate_cross_document_push_destination_surface_dest.html"
);
const NAVIGATION_ENTRIES_EXPOSE_CURRENT_ENTRY_METADATA_AND_IDENTITY_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/navigation_entries_expose_current_entry_metadata_and_identity.html"
);
const HISTORY_INITIAL_NAVIGATION_CURRENT_ENTRY_INDEX_STARTS_AT_ZERO_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_initial_navigation_current_entry_index_starts_at_zero.html"
);
const HISTORY_ONPOPSTATE_PROPERTY_RECEIVES_RESTORED_STATE_AFTER_BACK_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_onpopstate_property_receives_restored_state_after_back.html"
);
const HISTORY_BACK_FRAGMENT_TRAVERSAL_DISPATCHES_POPSTATE_THEN_HASHCHANGE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_back_fragment_traversal_dispatches_popstate_then_hashchange.html"
);
const HISTORY_FORWARD_FRAGMENT_TRAVERSAL_DISPATCHES_POPSTATE_THEN_HASHCHANGE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_forward_fragment_traversal_dispatches_popstate_then_hashchange.html"
);
const HISTORY_LOCATION_REPLACE_FRAGMENT_REPLACES_CURRENT_ENTRY_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_location_replace_fragment_replaces_current_entry.html"
);
const CANVAS_TO_DATA_URL_EXISTS_AND_HANDLES_ZERO_SIZE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/canvas_to_data_url_exists_and_handles_zero_size.html"
);
const EVENT_HANDLER_ACCESSORS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/event_handler_accessors.html");
const HTML_CONTENT_ACCESSORS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/html_content_accessors.html");
const DETAILS_DIALOG_ACCESSORS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/details_dialog_accessors.html");
const HTML_ELEMENT_REFLECTED_ACCESSORS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/html_element_reflected_accessors.html");
const STYLE_LINK_STYLESHEET_ACCESSORS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/style_link_stylesheet_accessors.html");
const SCRIPT_STATE_SNAPSHOT_HANDLES_THROWING_TO_PRIMITIVE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/script_state_snapshot_handles_throwing_to_primitive.html"
);
const SCRIPT_STATE_SNAPSHOT_IGNORES_SET_PROTOTYPE_TAMPER_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/script_state_snapshot_ignores_set_prototype_tamper.html"
);
const SHADOW_DOM_SLOT_TEMPLATE_ACCESSORS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/shadow_dom_slot_template_accessors.html");
const DOCUMENT_HAS_FOCUS_TOP_LEVEL_TRUE_CHILD_FALSE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_has_focus_top_level_true_child_false.html"
);
const HISTORY_INITIAL_CHILD_ENTRY_SEED_PARENT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_initial_child_entry_seed_parent.html"
);
const HISTORY_INITIAL_CHILD_ENTRY_SEED_CHILD_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/history_initial_child_entry_seed_child.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_FRAGMENT_TRAVERSAL_EVENTS_ARE_WINDOW_LOCAL_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_fragment_traversal_events_are_window_local.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_LOCATION_HASH_ASSIGNMENT_DISPATCHES_LOCAL_POPSTATE_AND_HASHCHANGE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_location_hash_assignment_dispatches_local_popstate_and_hashchange.html"
);
const WINDOW_CRYPTO_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/window_crypto.html");
const WINDOW_CSS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/window_css.html");
const SERVO_MATCH_MEDIA_PARSING_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_match_media_parsing.html");
const SERVO_STYLE_ATTR_BRACES_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_style_attr_braces.html");
const SERVO_STYLE_ATTR_URLS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_style_attr_urls.html");
const SERVO_QUERY_IS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_query_is.html");
const SERVO_QUERY_WHERE_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_query_where.html");
const SERVO_MATCH_MEDIA_CASE_INSENSITIVE_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_match_media_case_insensitive.html");
const SERVO_MATCH_MEDIA_INVALID_TYPES_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_match_media_invalid_types.html");
const SERVO_MATCH_MEDIA_FEATURE_STATES_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_match_media_feature_states.html");
const SERVO_MATCH_MEDIA_ASPECT_RATIO_SERIALIZATION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/servo_match_media_aspect_ratio_serialization.html"
);
const SERVO_MATCH_MEDIA_PREFERENCES_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_match_media_preferences.html");
const SERVO_MEDIA_QUERY_LIST_EVENT_TARGET_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_media_query_list_event_target.html");
const SERVO_CSS_SUPPORTS_CONDITIONS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_css_supports_conditions.html");
const SERVO_FONTFACESET_HISTORICAL_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_fontfaceset_historical.html");
const SERVO_FONTFACESET_CONNECTED_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_fontfaceset_connected.html");
const SERVO_FONTFACESET_CONNECTED_IGNORE_PAGE_TAMPERED_STYLE_QUERIES_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/servo_fontfaceset_connected_ignore_page_tampered_style_queries.html"
);
const SERVO_FONTFACESET_CONNECTED_CLEAR_DELETE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/servo_fontfaceset_connected_clear_delete.html"
);
const SERVO_FONTFACESET_HAS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_fontfaceset_has.html");
const SERVO_FONTFACESET_DELETE_CLEAR_CSS_CONNECTED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/servo_fontfaceset_delete_clear_css_connected.html"
);
const SERVO_FONTFACESET_LOAD_READY_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_fontfaceset_load_ready.html");
const SERVO_FONTFACESET_EMPTY_FAMILY_LOAD_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_fontfaceset_empty_family_load.html");
const SERVO_FONTFACESET_NO_ROOT_ELEMENT_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/servo_fontfaceset_no_root_element.html");
const SERVO_FONTFACESET_UPDATE_AFTER_STYLESHEET_CHANGE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/servo_fontfaceset_update_after_stylesheet_change.html"
);
const SERVO_FONTFACESET_LOAD_CSS_CONNECTED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/servo_fontfaceset_load_css_connected.html"
);
const CHROME_MEDIA_QUERY_LIST_ADD_REMOVE_LISTENER_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/chrome_media_query_list_add_remove_listener.html"
);
const CHROME_CSS_ESCAPE_DOM_API_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_css_escape_dom_api.html");
const CHROME_STYLESHEETLIST_STYLE_ONLY_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_stylesheetlist_style_only.html");
const CHROME_STYLESHEETLIST_MIXED_DISABLED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/chrome_stylesheetlist_mixed_disabled.html"
);
const CHROME_STYLESHEETLIST_ITEM_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_stylesheetlist_item.html");
const CHROME_CSSOM_MISSING_ARGUMENTS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_cssom_missing_arguments.html");
const CHROME_CSSFLOAT_CSSOM_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_cssfloat_cssom.html");
const CHROME_OVERFLOW_PROPERTY_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_overflow_property.html");
const CHROME_CSSSTYLESHEET_RULE_MUTATION_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_cssstylesheet_rule_mutation.html");
const CHROME_DELETE_RULE_NO_CRASH_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_delete_rule_no_crash.html");
const CHROME_IMPORTANT_JS_OVERRIDE_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_important_js_override.html");
const CHROME_BOX_SIZING_BACKWARDS_COMPAT_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_box_sizing_backwards_compat.html");
const CHROME_CSS_SUPPORTS_DOM_API_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_css_supports_dom_api.html");
const CHROME_CSS_SUPPORTS_SHORTHANDS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_css_supports_shorthands.html");
const CHROME_CSS_SUPPORTS_SYNTAX_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_css_supports_syntax.html");
const CHROME_CSS_SUPPORTS_COERCION_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_css_supports_coercion.html");
const CHROME_FONTFACESET_BASIC_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_fontfaceset_basic.html");
const CHROME_FONTFACESET_ITERATION_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_fontfaceset_iteration.html");
const CHROME_FONTFACESET_PLATFORM_FONTS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_fontfaceset_platform_fonts.html");
const CHROME_FONTFACESET_EVENTS_SUBSET_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_fontfaceset_events_subset.html");
const CHROME_FONTFACESET_SET_OPERATIONS_SUBSET_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/chrome_fontfaceset_set_operations_subset.html"
);
const CHROME_FONTFACESET_DETACHED_FRAME_READY_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/chrome_fontfaceset_detached_frame_ready.html"
);
const CHROME_FONTFACESET_READY_BASIC_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_fontfaceset_ready_basic.html");
const CHROME_FONTFACESET_INVALID_FAMILY_NAMES_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/chrome_fontfaceset_invalid_family_names.html"
);
const CHROME_FONTFACESET_UNATTACHED_DOCUMENT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/chrome_fontfaceset_unattached_document.html"
);
const CHROME_WEBFONT_INSERT_RULE_NO_CRASH_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/chrome_webfont_insert_rule_no_crash.html");
const WINDOW_CHILD_BROWSING_CONTEXT_TARGET_NAME_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_target_name.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_PARSE_TIME_TARGET_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_parse_time_target.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_FORM_TARGETS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_form_targets.html"
);
const DOCUMENT_FONTS_EVENTS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/document_fonts_events.html");
const WINDOW_HOST_GLOBALS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/window_host_globals.html");
const WINDOW_CHILD_BROWSING_CONTEXT_LENGTH_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_length.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_SNAPSHOT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_snapshot.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_POST_MESSAGE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_post_message.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_WINDOW_GRAPH_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_window_graph.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_RUNTIME_BACKING_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_runtime_backing.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_SCRIPT_GLOBALS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_script_globals.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_SCRIPT_GLOBALS_CHILD_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_script_globals_child.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_HISTORY_RELATIVE_URLS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_history_relative_urls.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_FRAGMENT_NAVIGATION_HISTORY_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_fragment_navigation_history.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_INITIAL_JOINT_HISTORY_TIMING_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_initial_joint_history_timing.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_LOCATION_NAVIGATION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_location_navigation.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_LOCATION_PATHNAME_PENDING_DOCUMENT_COHERENCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_location_pathname_pending_document_coherence.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_ATTRIBUTE_NAVIGATION_HISTORY_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_attribute_navigation_history.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_STATE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_state.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_BACK_CROSS_DOCUMENT_DESTINATION_SURFACE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_back_cross_document_destination_surface.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_NOOP_RESULT_SURFACE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_noop_result_surface.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_TRAVERSE_TO_NOOP_RESULT_SURFACE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_traverse_to_noop_result_surface.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_HISTORY_POPSTATE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_history_popstate.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_CURRENTENTRYCHANGE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_currententrychange.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_ACTIVATION_SAME_DOCUMENT_NAVIGATION_STAYS_INITIAL_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_activation_same_document_navigation_stays_initial.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_IFRAME_LOAD_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_iframe_load.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_IDENTITY_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_identity.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_REDIRECT_COHERENCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_redirect_coherence.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_DELAYED_ASYNC_NAVIGATION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_delayed_async_navigation.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_PENDING_NAVIGATION_COHERENCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_pending_navigation_coherence.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_DELAYED_EXTERNAL_SCRIPT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_delayed_external_script.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_STALE_ASYNC_NAVIGATION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_stale_async_navigation.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_STALE_EXTERNAL_SCRIPT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_stale_external_script.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_DISCONNECTED_ASYNC_NAVIGATION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_disconnected_async_navigation.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_POST_MESSAGE_ORIGIN_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_post_message_origin.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_WORKER_RELAY_PARENT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_worker_relay_parent.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_WORKER_RELAY_CHILD_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_worker_relay_child.html"
);
const NAVIGATOR_EXTENDED_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/navigator_extended.html");
const EVENT_BUBBLES_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/event_bubbles.html");
const EVENT_LISTENER_EXCEPTION_DISPATCH_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/event_listener_exception_dispatch.html");
const CUSTOM_ELEMENT_CALLBACK_EXCEPTION_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/custom_element_callback_exception.html");
const LOCAL_EVENT_TARGET_CALLBACK_EXCEPTION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/local_event_target_callback_exception.html"
);
const SYNC_FOREACH_CALLBACK_EXCEPTION_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/sync_foreach_callback_exception.html");
const WINDOW_NAMED_ACCESS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/window_named_access.html");
const WINDOW_MATCH_MEDIA_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/window_match_media.html");
const WINDOW_SCREEN_EVENTS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/window_screen_events.html");
const UNCAUGHT_SCRIPT_ERROR_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/uncaught_script_error.html");
const LOAD_LISTENER_ERROR_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/load_listener_error.html");
const HANDLED_PROMISE_REJECTION_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/handled_promise_rejection.html");
const UNHANDLED_PROMISE_REJECTION_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/unhandled_promise_rejection.html");
const CAUGHT_DYNAMIC_BARE_IMPORT_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/caught_dynamic_bare_import.html");
const QUEUE_MICROTASK_IGNORES_PROMISE_TAMPER_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/queue_microtask_ignores_promise_tamper.html"
);
const POST_MESSAGE_IGNORES_PAGE_TAMPERED_QUEUE_MICROTASK_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/post_message_ignores_page_tampered_queue_microtask.html"
);
const MESSAGE_PORT_IGNORES_PAGE_TAMPERED_QUEUE_MICROTASK_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/message_port_ignores_page_tampered_queue_microtask.html"
);
const MUTATION_OBSERVER_IGNORES_PAGE_TAMPERED_QUEUE_MICROTASK_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/mutation_observer_ignores_page_tampered_queue_microtask.html"
);
const MESSAGE_PORT_CALLBACK_ERROR_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/message_port_callback_error.html");
const FILE_READER_CALLBACK_ERROR_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/file_reader_callback_error.html");
const MUTATION_OBSERVER_CALLBACK_ERROR_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/mutation_observer_callback_error.html");
const RESIZE_OBSERVER_CALLBACK_ERROR_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/resize_observer_callback_error.html");
const XHR_IGNORES_PAGE_TAMPERED_QUEUE_MICROTASK_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/xhr_ignores_page_tampered_queue_microtask.html"
);
const XHR_CALLBACK_ERROR_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/xhr_callback_error.html");
const ABORT_SIGNAL_CALLBACK_ERROR_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/abort_signal_callback_error.html");
const DUMP_DOM_SNAPSHOT_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/dump_dom_snapshot.html");
const PARSE_TIME_INLINE_CLASSIC_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/parse_time_inline_classic.html");
const PARSE_TIME_EXTERNAL_CLASSIC_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/parse_time_external_classic.html");
const SCRIPT_SRC_BASE_ALPHA_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/script_src_base_alpha.html");
const PARSE_TIME_DEFER_CLASSIC_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/parse_time_defer_classic.html");
const PARSE_TIME_ASYNC_CLASSIC_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/parse_time_async_classic.html");
const PARSE_TIME_ASYNC_CLASSIC_CHUNKED_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/parse_time_async_classic_chunked.html");
const PARSE_TIME_ASYNC_CLASSIC_SLOW_CHUNKED_TAIL_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parse_time_async_classic_slow_chunked_tail.html"
);
const PARSE_TIME_ASYNC_CLASSIC_PUMPED_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/parse_time_async_classic_pumped.html");
const PARSE_TIME_ASYNC_CLASSIC_SLOW_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/parse_time_async_classic_slow.html");
const PARSE_TIME_ASYNC_CLASSIC_TASK_TURNS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/parse_time_async_classic_task_turns.html");
const PARSE_TIME_ASYNC_CLASSIC_TASK_TURN_VISIBILITY_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parse_time_async_classic_task_turn_visibility.html"
);
const PARSE_TIME_ASYNC_CLASSIC_POST_PARSE_TURNS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parse_time_async_classic_post_parse_turns.html"
);
const PARSE_TIME_ASYNC_CLASSIC_POST_PARSE_SLOW_SECOND_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parse_time_async_classic_post_parse_slow_second.html"
);
const BLOCKING_STYLESHEET_PARSER_BLOCKING_EXTERNAL_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/blocking_stylesheet_parser_blocking_external.html"
);
const BLOCKING_STYLESHEET_PARSER_BLOCKING_DOCUMENT_WRITE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/blocking_stylesheet_parser_blocking_document_write.html"
);
const BLOCKING_STYLESHEET_DEFER_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/blocking_stylesheet_defer.html");
const BLOCKING_STYLESHEET_MODULE_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/blocking_stylesheet_module.html");
const PHASE_TWO_UPGRADE_RUNTIME_STYLE_DEFER_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/phase_two_upgrade_runtime_style_defer.html"
);
const PHASE_TWO_UPGRADE_RUNTIME_STYLE_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/phase_two_upgrade_runtime_style_module.html"
);
const PHASE_TWO_SHARED_BLOCKING_STYLESHEET_DEFER_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/phase_two_shared_blocking_stylesheet_defer.html"
);
const PHASE_TWO_SHARED_BLOCKING_STYLESHEET_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/phase_two_shared_blocking_stylesheet_module.html"
);
const BLOCKING_STYLESHEET_PARSER_CREATED_STYLE_IMPORT_PARSER_BLOCKING_EXTERNAL_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/blocking_stylesheet_parser_created_style_import_parser_blocking_external.html"
);
const BLOCKING_STYLESHEET_PARSER_CREATED_STYLE_IMPORT_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/blocking_stylesheet_parser_created_style_import_module.html"
);
const BLOCKING_STYLESHEET_ALTERNATE_NON_BLOCKING_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/blocking_stylesheet_alternate_non_blocking.html"
);
const DOCUMENT_WRITE_IMPLICIT_REPLACE_DROPS_OLD_DEFER_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_implicit_replace_drops_old_defer.html"
);
const DOCUMENT_WRITE_IMPLICIT_REPLACE_DROPS_OLD_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_implicit_replace_drops_old_module.html"
);
const DOCUMENT_WRITE_REPLACEMENT_ASYNC_STAYS_AFTER_DOMCONTENTLOADED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_replacement_async_stays_after_domcontentloaded.html"
);
const DOCUMENT_WRITE_REPLACEMENT_STYLE_SOURCE_SYNC_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_replacement_style_source_sync.html"
);
const DOCUMENT_WRITE_NESTED_WRITER_RESTORES_OUTER_INSERTION_POINT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_nested_writer_restores_outer_insertion_point.html"
);
const DOCUMENT_WRITE_NESTED_EXTERNAL_SCRIPT_SERIALIZES_OUTER_RESUME_HTML: &str = r#"<!doctype html><html><head><script>window.documentWriteNestedExternalOrder=[];document.write('<script src="/assets/document_write_nested_external_parent.js"><\/script><script src="/assets/document_write_nested_external_outer_after.js"><\/script><script>window.documentWriteNestedExternalOrder.push("outer-inline");window.documentWriteNestedExternalResult=window.documentWriteNestedExternalOrder.join(",");<\/script>');</script></head><body><main id="after">after</main></body></html>"#;
const DOCUMENT_WRITE_EXTERNAL_SPLIT_SCRIPT_PARSER_SESSION_HTML: &str = r#"<!doctype html><html><head><script>window.documentWriteExternalSplitSessionOrder=[];</script><script src="/assets/document_write_external_split_session_parent.js"></script><script>window.documentWriteExternalSplitSessionOrder.push("after-parent");window.documentWriteExternalSplitSessionResult=window.documentWriteExternalSplitSessionOrder.join(",");</script></head><body><main id="tail">tail</main></body></html>"#;
const DOCUMENT_WRITE_INSERTED_EXTERNAL_CHUNKED_HEAD: &str = r#"<!doctype html><html><head><script>window.documentWriteInsertedChunkedOrder=["before-write"];document.write('<script src="/assets/document_write_inserted_chunked_external.js"><\/script>');window.documentWriteInsertedChunkedOrder.push("after-write");</script></head><body>"#;
const DOCUMENT_WRITE_INSERTED_EXTERNAL_CHUNKED_TAIL: &str = r#"<main id="document-write-inserted-chunked-tail">tail</main><script>window.documentWriteInsertedChunkedOrder.push("tail-script");window.documentWriteInsertedChunkedResult=window.documentWriteInsertedChunkedOrder.join(",");</script></body></html>"#;
const DOCUMENT_WRITE_PARSER_VISIBLE_DOM_BOUNDARY_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_parser_visible_dom_boundary.html"
);
const PARSER_OWNED_MODULE_PENDING_STAR_LINK_FAILURE_BEFORE_BODY_AND_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_owned_module_pending_star_link_failure_before_body_and_later_module.html"
);
const DYNAMIC_MODULE_PENDING_STAR_LINK_FAILURE_BEFORE_BODY_AND_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_module_pending_star_link_failure_before_body_and_later_module.html"
);
const PARSER_OWNED_MODULE_PENDING_STAR_FINAL_MISSING_REPORTS_LINK_FAILURE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_owned_module_pending_star_final_missing_reports_link_failure.html"
);
const DYNAMIC_MODULE_PENDING_STAR_FINAL_MISSING_REPORTS_LINK_FAILURE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_module_pending_star_final_missing_reports_link_failure.html"
);
const DOCUMENT_WRITE_EXTERNAL_PARSER_BLOCKING_BOUNDARY_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_external_parser_blocking_boundary.html"
);
const DOCUMENT_WRITE_EXTERNAL_SCRIPT_LOAD_MICROTASK_BEFORE_LATER_WRITTEN_INLINE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_external_script_load_microtask_before_later_written_inline.html"
);
const DOCUMENT_WRITE_IMPORTMAP_BEFORE_WRITTEN_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_importmap_before_written_module.html"
);
const DOCUMENT_WRITE_IMPORTMAP_BEFORE_WRITTEN_EXTERNAL_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_importmap_before_written_external_module.html"
);
const DOCUMENT_WRITE_INVALID_IMPORTMAP_BEFORE_WRITTEN_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_invalid_importmap_before_written_module.html"
);
const DOCUMENT_WRITE_INVALID_IMPORTMAP_BEFORE_RESTORE_INLINE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_invalid_importmap_before_restore_inline.html"
);
const DOCUMENT_WRITE_DEFER_QUEUES_AFTER_LATER_CLASSIC_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_defer_queues_after_later_classic.html"
);
const DOCUMENT_WRITE_DEFER_RUNS_BEFORE_DOMCONTENTLOADED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_defer_runs_before_domcontentloaded.html"
);
const CHILD_DOCUMENT_OPEN_AFTER_PARENT_LOAD_DATA_SCRIPT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/child_document_open_after_parent_load_data_script.html"
);
const IMPORTED_STARTED_CHILD_SCRIPT_STAYS_INERT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/imported_started_child_script_stays_inert.html"
);
const DOCUMENT_OPEN_AFTER_LOAD_EXTERNAL_SCRIPTS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_open_after_load_external_scripts.html"
);
const DOCUMENT_WRITE_MULTI_LEVEL_NESTED_WRITER_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_multi_level_nested_writer.html"
);
const DOCUMENT_WRITE_LATE_STYLESHEET_DOES_NOT_BLOCK_WRITTEN_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_late_stylesheet_does_not_block_written_module.html"
);
const DOCUMENT_WRITE_SPLIT_TAGS_STREAM_ACROSS_CALLS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_split_tags_stream_across_calls.html"
);
const DOCUMENT_WRITE_SPLIT_SCRIPT_STREAM_ACROSS_CALLS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_split_script_stream_across_calls.html"
);
const DOCUMENT_WRITE_SPLIT_EXTERNAL_SCRIPT_STREAM_ACROSS_CALLS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_split_external_script_stream_across_calls.html"
);
const DOCUMENT_WRITE_SPLIT_IMPORTMAP_AND_MODULE_STREAM_ACROSS_CALLS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_split_importmap_and_module_stream_across_calls.html"
);
const RUST_DOM_DOCUMENT_OPEN_MULTIWRITE_SYNC_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/rust_dom_document_open_multiwrite_sync.html"
);
const RUNTIME_INSERTED_STYLESHEET_LOAD_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/runtime_inserted_stylesheet_load.html");
const RUNTIME_INSERTED_STYLESHEET_LOAD_SYNCS_PARSER_SNAPSHOT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_inserted_stylesheet_load_syncs_parser_snapshot.html"
);
const RUNTIME_INSERTED_STYLESHEET_LOAD_TRIGGERS_LOCATION_REPLACE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_inserted_stylesheet_load_triggers_location_replace.html"
);
const RUNTIME_INSERTED_STYLESHEET_HREF_MUTATION_USES_FRESH_FETCH_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_inserted_stylesheet_href_mutation_uses_fresh_fetch.html"
);
const RUNTIME_INSERTED_STYLE_IMPORT_MISSING_COMPLETES_LOAD_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_inserted_style_import_missing_completes_load.html"
);
const DYNAMIC_SCRIPT_WAITS_FOR_RUNTIME_INSERTED_STYLESHEET_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_script_waits_for_runtime_inserted_stylesheet.html"
);
const RUNTIME_INSERTED_PRELOAD_AND_MODULEPRELOAD_PARSER_PROGRESS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_inserted_preload_and_modulepreload_parser_progress.html"
);
const MODULEPRELOAD_SHARED_STATIC_DEPENDENCY_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/modulepreload_shared_static_dependency.html"
);
const MODULEPRELOAD_DUPLICATE_SHARED_STATIC_DEPENDENCY_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/modulepreload_duplicate_shared_static_dependency.html"
);
const DUPLICATE_MODULE_ROOT_EVAL_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/duplicate_module_root_eval.html");
const DUPLICATE_MODULE_ROOT_WITH_NESTED_DEPENDENCIES_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/duplicate_module_root_with_nested_dependencies.html"
);
const MODULE_TOP_LEVEL_FETCH_AND_MIME_ERRORS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_top_level_fetch_and_mime_errors_dispatch_script_error.html"
);
const MODULEPRELOAD_REUSED_PARENT_PENDING_CHILD_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/modulepreload_reused_parent_pending_child.html"
);
const DYNAMIC_SCRIPT_ASYNC_OVERTAKES_IN_ORDER_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_script_async_overtakes_in_order.html"
);
const DYNAMIC_SCRIPT_IN_ORDER_PRESERVES_ORDER_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_script_in_order_preserves_order.html"
);
const PARSE_TIME_DYNAMIC_SCRIPT_LOAD_AFTER_PARSER_PROGRESS_HTML: &str = "<!doctype html><html><head><script>window.parseTimeDynamicLoadOrder=[];window.parseTimeDynamicSaw='not-run';window.parseTimeDynamicUnsafeMarker='not-run';window.parseTimeDynamicLastError='';window.page={comm:{invokeApps:{marker:'initial'}}};window.addEventListener('error',event=>{const message=event&&event.message?String(event.message):'';window.parseTimeDynamicLastError=message;});</script><script src=\"/assets/parse_time_dynamic_clobber.js\"></script><script>window.parseTimeDynamicLoadOrder.push('restore-inline');window.page.comm={invokeApps:{marker:'restored'}};window.parseTimeDynamicRestored=window.page.comm.invokeApps.marker;window.parseTimeDynamicRestoreOrder=window.parseTimeDynamicLoadOrder.join(',');</script></head><body><main id=\"after\">after</main></body></html>";
const PARSE_TIME_DYNAMIC_SCRIPT_ERROR_AFTER_PARSER_PROGRESS_HTML: &str = "<!doctype html><html><head><script>window.parseTimeDynamicErrorOrder=[];window.parseTimeDynamicErrorSaw='not-run';window.page={comm:{invokeApps:{marker:'initial'}}};</script><script src=\"/assets/parse_time_dynamic_error_clobber.js\"></script><script>window.parseTimeDynamicErrorOrder.push('restore-inline');window.page.comm={invokeApps:{marker:'restored'}};window.parseTimeDynamicErrorRestored=window.page.comm.invokeApps.marker;window.parseTimeDynamicErrorRestoreOrder=window.parseTimeDynamicErrorOrder.join(',');</script></head><body><main id=\"after\">after</main></body></html>";
const PARSER_CONNECTED_EXTERNAL_CLASSIC_DISPATCHES_LOAD_HTML: &str = "<!doctype html><html><head><script>window.parserConnectedLoadOrder=[];window.parserConnectedLoadSaw='not-fired';</script><script src=\"/assets/parse_time_classic.js\" onload=\"window.parserConnectedLoadOrder.push('external-load');window.parserConnectedLoadSaw='fired';window.parserConnectedLoadFinalOrder=window.parserConnectedLoadOrder.join(',');\"></script><script>window.parserConnectedLoadOrder.push('after-external');window.parserConnectedLoadAfterExternalOrder=window.parserConnectedLoadOrder.join(',');</script></head><body><main id=\"after\">after</main></body></html>";
const PARSER_CONNECTED_EXTERNAL_CLASSIC_LOAD_DOCUMENT_WRITE_INSERTION_POINT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_connected_external_classic_load_document_write_insertion_point.html"
);
const PARSER_CONNECTED_EXTERNAL_CLASSIC_LOAD_DOCUMENT_WRITE_PARENT_CALLBACK_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_connected_external_classic_load_document_write_parent_callback.html"
);
const PARSER_CONNECTED_EXTERNAL_CLASSIC_LOAD_DOCUMENT_WRITE_PARENT_CALLBACK_CHILD_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_connected_external_classic_load_document_write_parent_callback_child.html"
);
const PARSER_CONNECTED_EXTERNAL_CLASSIC_ERROR_MICROTASK_HTML: &str = "<!doctype html><html><head><script>window.parserConnectedErrorOrder=[];window.parserConnectedErrorSaw='not-fired';window.parserConnectedErrorDuring='not-fired';window.parserConnectedErrorAfterInline='not-fired';</script><script src=\"/assets/missing_parse_time_classic.js\" onerror=\"window.parserConnectedErrorOrder.push('error');window.parserConnectedErrorSaw='fired';window.parserConnectedErrorDuring=window.parserConnectedErrorOrder.join(',');queueMicrotask(()=>{window.parserConnectedErrorOrder.push('error-microtask');window.parserConnectedErrorMicrotaskOrder=window.parserConnectedErrorOrder.join(',');});\"></script><script>window.parserConnectedErrorOrder.push('after-inline');window.parserConnectedErrorAfterInline=window.parserConnectedErrorOrder.join(',');</script></head><body><main id=\"after\">after</main><script>window.parserConnectedErrorFinalOrder=window.parserConnectedErrorOrder.join(',');</script></body></html>";
const PARSER_CONNECTED_EXTERNAL_CLASSIC_UNKNOWN_SCHEME_ERRORS_AND_CONTINUES_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_connected_external_classic_unknown_scheme_errors_and_continues.html"
);
const PARSER_CONNECTED_INLINE_CLASSIC_DOES_NOT_DISPATCH_LOAD_HTML: &str = "<!doctype html><html><head><script id=\"inline-current\">window.parserInlineLoadTargets=[];window.parserInlineLoadSaw='not-fired';const dynamic=document.createElement('script');document.documentElement.addEventListener('load',event=>{window.parserInlineLoadTargets.push(event.target===dynamic?'dynamic':(event.target.id||event.target.getAttribute('src')||'unknown'));window.parserInlineLoadFinalTargets=window.parserInlineLoadTargets.join(',');},true);dynamic.onload=()=>{window.parserInlineLoadSaw='fired';window.parserInlineLoadAttributeTarget='dynamic';};dynamic.src='/assets/app.js';document.head.appendChild(dynamic);</script></head><body><main id=\"after\">after</main></body></html>";
const PARSER_OWNED_EXTERNAL_DEFER_DISPATCHES_LOAD_HTML: &str = "<!doctype html><html><head><script>window.parserOwnedDeferLoadOrder=[];window.parserOwnedDeferLoadSaw='not-fired';</script><script defer src=\"/assets/parse_time_defer.js\" onload=\"window.parserOwnedDeferLoadOrder.push('external-load');window.parserOwnedDeferLoadSaw='fired';window.parserOwnedDeferLoadFinalOrder=window.parserOwnedDeferLoadOrder.join(',');\"></script><script defer>window.parserOwnedDeferLoadOrder.push('inline-defer');window.parserOwnedDeferLoadAfterInlineOrder=window.parserOwnedDeferLoadOrder.join(',');</script></head><body><main id=\"after\">after</main></body></html>";
const PARSER_OWNED_EXTERNAL_ASYNC_DISPATCHES_LOAD_HTML: &str = "<!doctype html><html><head><script>window.parserOwnedAsyncLoadOrder=[];window.parserOwnedAsyncDclFired=false;document.addEventListener('DOMContentLoaded',()=>{window.parserOwnedAsyncDclFired=true;window.parserOwnedAsyncLoadOrder.push('dcl');window.parserOwnedAsyncLoadAfterDclOrder=window.parserOwnedAsyncLoadOrder.join(',');});window.addEventListener('load',()=>{window.parserOwnedAsyncLoadOrder.push('window-load');window.parserOwnedAsyncLoadWindowOrder=window.parserOwnedAsyncLoadOrder.join(',');});</script><script async src=\"/assets/parse_time_async_load_order.js\" onload=\"window.parserOwnedAsyncLoadOrder.push('script-load');window.parserOwnedAsyncLoadSaw='fired';window.parserOwnedAsyncLoadSawTail=!!document.getElementById('late');window.parserOwnedAsyncLoadSawDcl=window.parserOwnedAsyncDclFired===true;window.parserOwnedAsyncLoadFinalOrder=window.parserOwnedAsyncLoadOrder.join(',');\"></script></head><body><main id=\"late\">late</main></body></html>";
const RUNTIME_OWNED_EXTERNAL_IN_ORDER_LOAD_AFTER_DOMCONTENTLOADED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_owned_external_in_order_load_after_domcontentloaded.html"
);
const RUNTIME_OWNED_EXTERNAL_IN_ORDER_WITH_DEFER_STAYS_AFTER_DOMCONTENTLOADED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_owned_external_in_order_with_defer_stays_after_domcontentloaded.html"
);
const RUNTIME_OWNED_EXTERNAL_IN_ORDER_ERROR_AFTER_DOMCONTENTLOADED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_owned_external_in_order_error_after_domcontentloaded.html"
);
const RUNTIME_OWNED_EXTERNAL_IN_ORDER_FROM_DOMCONTENTLOADED_HANDLER_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_owned_external_in_order_from_domcontentloaded_handler.html"
);
const RUNTIME_OWNED_EXTERNAL_ASYNC_DOES_NOT_BLOCK_DOMCONTENTLOADED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_owned_external_async_does_not_block_domcontentloaded.html"
);
const RUNTIME_OWNED_EXTERNAL_ASYNC_FAST_DOES_NOT_OVERTAKE_DOMCONTENTLOADED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_owned_external_async_fast_does_not_overtake_domcontentloaded.html"
);
const RUNTIME_OWNED_EXTERNAL_ASYNC_WITH_DEFER_DOES_NOT_BLOCK_DOMCONTENTLOADED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_owned_external_async_with_defer_does_not_block_domcontentloaded.html"
);
const RUNTIME_OWNED_DEFAULT_ASYNC_MODULE_SIDE_EFFECT_AFTER_DOMCONTENTLOADED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_owned_default_async_module_side_effect_after_domcontentloaded.html"
);
const RUNTIME_OWNED_INLINE_MODULE_SINGLE_LINE_IMPORT_EXECUTES_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_owned_inline_module_single_line_import_executes.html"
);
const RUNTIME_OWNED_INLINE_MODULE_RUNS_WHILE_PARSER_DEFER_IS_BLOCKED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_owned_inline_module_runs_while_parser_defer_is_blocked.html"
);
const RUNTIME_OWNED_INLINE_MODULE_MISSING_DEFAULT_EXPORT_AFTER_DOMCONTENTLOADED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_owned_inline_module_missing_default_export_after_domcontentloaded.html"
);
const RUNTIME_OWNED_EXTERNAL_MODULE_LOAD_FAILURE_AFTER_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_owned_external_module_load_failure_after_later_module.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_RESULT_SURFACE_IN_CHILD_SCRIPT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_result_surface_in_child_script.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_RESULT_SURFACE_SOURCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_result_surface_source.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_RESULT_SURFACE_DESTINATION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_result_surface_destination.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_PENDING_DOCUMENT_COHERENCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_pending_document_coherence.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_PUSH_STATE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_push_state.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_SAME_DOCUMENT_PUSH_RESULT_SURFACE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_same_document_push_result_surface.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_SAME_DOCUMENT_PUSH_RESULT_SURFACE_CHILD_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_same_document_push_result_surface_child.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_SAME_DOCUMENT_REPLACE_RESULT_SURFACE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_same_document_replace_result_surface.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_SAME_DOCUMENT_REPLACE_RESULT_SURFACE_CHILD_HTML:
    &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_same_document_replace_result_surface_child.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_PUSH_RESULT_SURFACE_IN_CHILD_SCRIPT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_push_result_surface_in_child_script.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_PUSH_RESULT_SURFACE_SOURCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_push_result_surface_source.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_PUSH_RESULT_SURFACE_DESTINATION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_push_result_surface_destination.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_RELOAD_RESULT_SURFACE_IN_CHILD_SCRIPT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_reload_result_surface_in_child_script.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_RELOAD_RESULT_SURFACE_CHILD_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_reload_result_surface_child.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_RELOAD_PENDING_DOCUMENT_COHERENCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_reload_pending_document_coherence.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_LOCATION_RELOAD_PENDING_DOCUMENT_COHERENCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_location_reload_pending_document_coherence.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_HISTORY_GO_ZERO_PENDING_DOCUMENT_COHERENCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_history_go_zero_pending_document_coherence.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_TRAVERSAL_PENDING_DOCUMENT_COHERENCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_traversal_pending_document_coherence.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_FORWARD_TRAVERSAL_PENDING_DOCUMENT_COHERENCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_forward_traversal_pending_document_coherence.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_CURRENT_ENTRY_SAME_DOCUMENT_USES_CHILD_OWNER_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_current_entry_same_document_uses_child_owner.html"
);
const RUNTIME_INSERTED_INLINE_SCRIPT_DOES_NOT_DISPATCH_LOAD_HTML: &str = "<!doctype html><html><head><script>window.runtimeInlineLoadOrder=[];window.runtimeInlineLoadSaw='not-fired';const script=document.createElement('script');script.text=\"window.runtimeInlineLoadOrder.push('script-run');\";script.onload=()=>{window.runtimeInlineLoadOrder.push('script-load');window.runtimeInlineLoadSaw='fired';window.runtimeInlineLoadOrderResult=window.runtimeInlineLoadOrder.join(',');};document.head.appendChild(script);window.runtimeInlineLoadOrder.push('after-append');window.runtimeInlineLoadOrderResult=window.runtimeInlineLoadOrder.join(',');</script></head><body><main id=\"after\">after</main></body></html>";
const DOCUMENT_WRITE_EXTERNAL_SCRIPT_LOAD_AFTER_PAGE_TASK_TURN_HTML: &str = r#"<!doctype html><html><head><script>window.documentWritePageTaskOrder=[];window.documentWritePageTaskSaw='not-run';window.page={comm:{invokeApps:{marker:'initial'}}};document.write('<script src="/assets/document_write_page_task_clobber.js" onload="window.documentWritePageTaskOrder.push(\'written-load\');window.documentWritePageTaskSaw=window.page&&window.page.comm&&window.page.comm.invokeApps?window.page.comm.invokeApps.marker:\'missing\';window.documentWritePageTaskFinalOrder=window.documentWritePageTaskOrder.join(\',\');"><\/script><script>window.documentWritePageTaskOrder.push(\'restore-inline\');window.page.comm={invokeApps:{marker:\'restored\'}};window.documentWritePageTaskRestored=window.page.comm.invokeApps.marker;window.documentWritePageTaskRestoreOrder=window.documentWritePageTaskOrder.join(\',\');<\/script>');</script></head><body><main id="after">after</main></body></html>"#;
const DOCUMENT_WRITE_EXTERNAL_SCRIPT_ERROR_AFTER_PAGE_TASK_TURN_HTML: &str = r#"<!doctype html><html><head><script>window.documentWriteErrorPageTaskOrder=[];window.documentWriteErrorPageTaskSaw='not-run';window.page={comm:{invokeApps:{marker:'initial'}}};document.write('<script src="/assets/missing_document_write_page_task.js" onerror="window.documentWriteErrorPageTaskOrder.push(\'written-error\');window.documentWriteErrorPageTaskSaw=window.page&&window.page.comm&&window.page.comm.invokeApps?window.page.comm.invokeApps.marker:\'missing\';window.documentWriteErrorPageTaskFinalOrder=window.documentWriteErrorPageTaskOrder.join(\',\');"><\/script><script>window.documentWriteErrorPageTaskOrder.push(\'restore-inline\');window.page.comm={invokeApps:{marker:\'restored\'}};window.documentWriteErrorPageTaskRestored=window.page.comm.invokeApps.marker;window.documentWriteErrorPageTaskRestoreOrder=window.documentWriteErrorPageTaskOrder.join(\',\');<\/script>');</script></head><body><main id="after">after</main></body></html>"#;
const DOCUMENT_WRITE_DELAYED_EXTERNAL_SCRIPT_DOES_NOT_BLOCK_PARENT_RUNTIME_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_delayed_external_script_does_not_block_parent_runtime.html"
);
const DOCUMENT_OPEN_DURING_PARSER_SCRIPT_WITH_PENDING_WRITTEN_EXTERNAL_IS_IGNORED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_open_during_parser_script_with_pending_written_external_is_ignored.html"
);
const DYNAMIC_SCRIPT_TYPE_MUTATION_REMAINS_INERT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_script_type_mutation_remains_inert.html"
);
const DYNAMIC_SCRIPT_REATTACH_STAYS_STARTED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_script_reattach_stays_started.html"
);
const DYNAMIC_SCRIPT_SRC_MUTATION_STAYS_STARTED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_script_src_mutation_stays_started.html"
);
const DYNAMIC_SCRIPT_ASYNC_ATTR_CLEARS_FORCE_ASYNC_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_script_async_attr_clears_force_async.html"
);
const DYNAMIC_SCRIPT_SRC_ADDED_AFTER_CONNECT_STARTS_ONCE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_script_src_added_after_connect_starts_once.html"
);
const DYNAMIC_SCRIPT_ERROR_DOES_NOT_ABORT_QUEUE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_script_error_does_not_abort_queue.html"
);
const DYNAMIC_SCRIPT_PREPARATION_CONTEXT_STAYS_IN_OLD_DOCUMENT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_script_preparation_context_stays_in_old_document.html"
);
const DYNAMIC_IMPORTMAP_BEFORE_MODULE_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/dynamic_importmap_before_module.html");
const DYNAMIC_ASYNC_MODULE_CLOSES_IMPORTMAP_ACQUISITION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_async_module_closes_importmap_acquisition.html"
);
const DYNAMIC_EXTERNAL_IMPORTMAP_ERROR_BEFORE_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_external_importmap_error_before_module.html"
);
const DYNAMIC_MODULE_EXECUTION_FAILURE_DOES_NOT_ABORT_QUEUE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_module_execution_failure_does_not_abort_queue.html"
);
const DYNAMIC_MODULE_TLA_REJECTION_DOES_NOT_ABORT_QUEUE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_module_tla_rejection_does_not_abort_queue.html"
);
const DYNAMIC_MODULE_TLA_EXOTIC_REJECTION_REPORTS_WINDOW_ERROR_PAYLOAD_DOES_NOT_ABORT_QUEUE_HTML:
    &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_module_tla_exotic_rejection_reports_window_error_payload_does_not_abort_queue.html"
);
const IMPORTMAP_SCOPES_AND_PREFIXES_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/importmap_scopes_and_prefixes.html");
const IMPORTMAP_MERGE_AFTER_RESOLUTION_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/importmap_merge_after_resolution.html");
const IMPORTMAP_URL_LIKE_NORMALIZATION_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/importmap_url_like_normalization.html");
const IMPORTMAP_AFTER_MODULE_LOAD_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/importmap_after_module_load.html");
const IMPORTMAP_CLOSED_BY_PARSER_OWNED_MODULE_BEFORE_LATE_DYNAMIC_MAP_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/importmap_closed_by_parser_owned_module_before_late_dynamic_map.html"
);
const PARSER_OWNED_IMPORTMAP_BLOCKED_AFTER_DYNAMIC_MODULE_PREPARE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_owned_importmap_blocked_after_dynamic_module_prepare.html"
);
const IMPORTMAP_NULL_BLOCKS_DYNAMIC_IMPORT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/importmap_null_blocks_dynamic_import.html"
);
const MODULE_BARE_SPECIFIER_WITHOUT_IMPORTMAP_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_bare_specifier_without_importmap.html"
);
const MODULE_DEFAULT_AND_SIDE_EFFECT_IMPORTS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_default_and_side_effect_imports.html"
);
const MODULE_DEFAULT_REEXPORT_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_default_reexport.html");
const MODULE_STRING_LITERAL_EXPORT_NAMES_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_string_literal_export_names.html");
const MODULE_STRING_LITERAL_EXPORT_NAMES_SURROGATE_PAIRS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_string_literal_export_names_surrogate_pairs.html"
);
const MODULE_EXPORT_STAR_STRING_LITERAL_NAMESPACE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_export_star_string_literal_namespace.html"
);
const MODULE_ESCAPED_IDENTIFIER_NAMES_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_escaped_identifier_names.html");
const MODULE_EXPORT_DEFAULT_FUNCTION_AND_CLASS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_export_default_function_and_class.html"
);
const MODULE_EXPORT_DEFAULT_ANONYMOUS_DECLARATIONS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_export_default_anonymous_declarations.html"
);
const MODULE_EXPORT_CLASS_NAMED_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_export_class_named.html");
const MODULE_EXPORT_GENERATOR_FUNCTIONS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_export_generator_functions.html");
const MODULE_EXPORT_CONST_MULTIPLE_BINDINGS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_export_const_multiple_bindings.html"
);
const MODULE_EXPORT_DESTRUCTURING_BINDINGS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_export_destructuring_bindings.html"
);
const MODULE_EXPORT_NESTED_DESTRUCTURING_BINDINGS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_export_nested_destructuring_bindings.html"
);
const MODULE_EXPORT_NESTED_INITIALIZER_COMMAS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_export_nested_initializer_commas.html"
);
const MODULE_IMPORT_EXPORT_LIST_COMMENTS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_import_export_list_comments.html");
const MODULE_MULTILINE_DYNAMIC_IMPORT_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_multiline_dynamic_import.html");
const MODULE_DYNAMIC_IMPORT_COMMENTS_AND_TRAILING_COMMA_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_dynamic_import_comments_and_trailing_comma.html"
);
const MODULE_DYNAMIC_IMPORT_STATIC_CONCAT_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_dynamic_import_static_concat.html");
const MODULE_IMPORT_ATTRIBUTES_AND_DYNAMIC_OPTIONS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_import_attributes_and_dynamic_options.html"
);
const MODULE_IMPORT_ASSERTIONS_LEGACY_SYNTAX_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_import_assertions_legacy_syntax.html"
);
const MODULE_IMPORT_META_RESOLVE_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_import_meta_resolve.html");
const MODULE_IMPORT_META_RESOLVE_COMMENTS_AND_TRAILING_COMMA_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_import_meta_resolve_comments_and_trailing_comma.html"
);
const MODULE_DYNAMIC_IMPORT_TEMPLATE_LITERAL_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_dynamic_import_template_literal.html"
);
const MODULE_ESCAPED_STRING_LITERAL_SPECIFIERS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_escaped_string_literal_specifiers.html"
);
const MODULE_EXPORT_VARIABLE_LIVE_BINDINGS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_export_variable_live_bindings.html"
);
const MODULE_SELF_BARE_DYNAMIC_IMPORT_RESOLVES_AFTER_OWN_EVALUATION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_self_bare_dynamic_import_resolves_after_own_evaluation.html"
);
const MODULE_SELF_BARE_DYNAMIC_IMPORT_AFTER_SETTLE_RESOLVES_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_self_bare_dynamic_import_after_settle_resolves.html"
);
const MODULE_RUNTIME_HELPER_SHADOWING_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_runtime_helper_shadowing.html");
const MODULE_CYCLE_DYNAMIC_IMPORT_WAITS_FOR_TARGET_EVALUATION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_dynamic_import_waits_for_target_evaluation.html"
);
const MODULE_CYCLE_EXPORT_STAR_LATE_BINDING_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_late_binding.html"
);
const MODULE_CYCLE_EXPORT_STAR_MULTIHOP_LATE_BINDING_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_multihop_late_binding.html"
);
const MODULE_CYCLE_EXPORT_STAR_LATE_AMBIGUOUS_BEFORE_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_late_ambiguous_before_later_module.html"
);
const MODULE_CYCLE_EXPORT_STAR_LATE_AMBIGUOUS_NAMESPACE_OMITS_EXPORT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_late_ambiguous_namespace_omits_export.html"
);
const MODULE_CYCLE_EXPORT_STAR_MULTIHOP_LATE_AMBIGUOUS_NAMESPACE_OMITS_EXPORT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_multihop_late_ambiguous_namespace_omits_export.html"
);
const MODULE_CYCLE_EXPORT_STAR_MULTIHOP_LATE_AMBIGUOUS_BEFORE_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_multihop_late_ambiguous_before_later_module.html"
);
const MODULE_STATIC_IMPORT_WAITS_FOR_INITIALIZING_NON_CYCLE_DEPENDENCY_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_static_import_waits_for_initializing_non_cycle_dependency.html"
);
const MODULE_EXPORT_STAR_AMBIGUOUS_BEFORE_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_export_star_ambiguous_before_later_module.html"
);
const MODULE_CYCLE_MISSING_EXPORT_BEFORE_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_missing_export_before_later_module.html"
);
const MODULE_CYCLE_INITIALIZING_MISSING_EXPORT_BEFORE_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_initializing_missing_export_before_later_module.html"
);
const MODULE_CYCLE_DEFAULT_MISSING_FROM_EXPORT_STAR_BEFORE_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_default_missing_from_export_star_before_later_module.html"
);
const PARSER_OWNED_MODULE_MISSING_EXPORT_REPORTS_WINDOW_ERROR_AFTER_RESTORE_INLINE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_owned_module_missing_export_reports_window_error_after_restore_inline.html"
);
const PARSER_OWNED_MODULE_TLA_REJECTION_REPORTS_WINDOW_ERROR_AFTER_RESTORE_INLINE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_owned_module_tla_rejection_reports_window_error_after_restore_inline.html"
);
const DOCUMENT_WRITE_MODULE_MISSING_EXPORT_REPORTS_WINDOW_ERROR_AFTER_RESTORE_INLINE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_module_missing_export_reports_window_error_after_restore_inline.html"
);
const DOCUMENT_WRITE_MODULE_TLA_REJECTION_REPORTS_WINDOW_ERROR_AFTER_RESTORE_INLINE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_module_tla_rejection_reports_window_error_after_restore_inline.html"
);
const PARSER_OWNED_MODULE_PENDING_STAR_MISSING_EXPORT_BEFORE_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_owned_module_pending_star_missing_export_before_later_module.html"
);
const PARSER_OWNED_MODULE_ERROR_BEFORE_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_owned_module_error_before_later_module.html"
);
const PARSER_OWNED_MODULE_MISSING_EXPORT_BEFORE_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_owned_module_missing_export_before_later_module.html"
);
const PARSER_OWNED_MODULE_MISSING_EXPORT_REPORTS_WINDOW_ERROR_BEFORE_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_owned_module_missing_export_reports_window_error_before_later_module.html"
);
const PARSER_OWNED_MODULE_MISSING_EXPORT_REPORTS_WINDOW_ERROR_PAYLOAD_BEFORE_LATER_MODULE_HTML:
    &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_owned_module_missing_export_reports_window_error_payload_before_later_module.html"
);
const PARSER_OWNED_MODULE_TLA_REJECTION_BEFORE_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_owned_module_tla_rejection_before_later_module.html"
);
const PARSER_OWNED_MODULE_TLA_EXOTIC_REJECTION_REPORTS_WINDOW_ERROR_PAYLOAD_BEFORE_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_owned_module_tla_exotic_rejection_reports_window_error_payload_before_later_module.html"
);
const PARSER_OWNED_IMPORTMAP_ERROR_BEFORE_LATER_MODULE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_owned_importmap_error_before_later_module.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_HISTORY_GO_ONE_FRAGMENT_TRAVERSAL_EVENTS_ARE_WINDOW_LOCAL_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_history_go_one_fragment_traversal_events_are_window_local.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_FORWARD_CURRENTENTRYCHANGE_TRAVERSE_EVENT_SURFACE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_forward_currententrychange_traverse_event_surface.html"
);
const WINDOW_CHILD_BROWSING_CONTEXT_NAVIGATION_FORWARD_RESULT_PROMISES_ARE_WINDOW_LOCAL_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/window_child_browsing_context_navigation_forward_result_promises_are_window_local.html"
);
const PARSER_OWNED_IMPORTMAP_ERROR_AFTER_PARSER_PROGRESS_HTML: &str = "<!doctype html><html><head><script>window.parserOwnedImportMapProgressOrder=[];window.parserOwnedImportMapProgressSaw='not-run';window.page={comm:{invokeApps:{marker:'initial'}}};window.addEventListener('error',()=>{window.parserOwnedImportMapProgressOrder.push('window-error');queueMicrotask(()=>{window.parserOwnedImportMapProgressOrder.push('window-error-microtask');window.parserOwnedImportMapProgressMicrotaskOrder=window.parserOwnedImportMapProgressOrder.join(',');window.parserOwnedImportMapProgressFinalOrder=window.parserOwnedImportMapProgressOrder.join(',');});window.parserOwnedImportMapProgressFinalOrder=window.parserOwnedImportMapProgressOrder.join(',');});</script><script type=\"importmap\" onerror=\"window.parserOwnedImportMapProgressOrder.push('error');window.parserOwnedImportMapProgressSaw=window.page&&window.page.comm&&window.page.comm.invokeApps?window.page.comm.invokeApps.marker:'missing';window.parserOwnedImportMapProgressFinalOrder=window.parserOwnedImportMapProgressOrder.join(',');\">{\"imports\":{\"fixture\":\"/assets/module-export-const-multiple-bindings.mjs\",}}</script><script>window.parserOwnedImportMapProgressOrder.push('restore-inline');window.page.comm={invokeApps:{marker:'restored'}};window.parserOwnedImportMapProgressRestored=window.page.comm.invokeApps.marker;window.parserOwnedImportMapProgressRestoreOrder=window.parserOwnedImportMapProgressOrder.join(',');window.parserOwnedImportMapProgressFinalOrder=window.parserOwnedImportMapProgressOrder.join(',');</script></head><body><main id=\"after\">after</main></body></html>";
const DYNAMIC_SCRIPT_NOMODULE_COMMITS_SKIP_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_script_nomodule_commits_skip.html"
);
const DYNAMIC_MODULE_MISSING_DEFAULT_EXPORT_DOES_NOT_ABORT_QUEUE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_module_missing_default_export_does_not_abort_queue.html"
);
const DYNAMIC_MODULE_MISSING_DEFAULT_EXPORT_REPORTS_WINDOW_ERROR_DOES_NOT_ABORT_QUEUE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_module_missing_default_export_reports_window_error_does_not_abort_queue.html"
);
const DYNAMIC_MODULE_MISSING_DEFAULT_EXPORT_REPORTS_WINDOW_ERROR_PAYLOAD_DOES_NOT_ABORT_QUEUE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_module_missing_default_export_reports_window_error_payload_does_not_abort_queue.html"
);
const DYNAMIC_MODULE_PENDING_STAR_MISSING_EXPORT_DOES_NOT_ABORT_QUEUE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_module_pending_star_missing_export_does_not_abort_queue.html"
);
const PARSE_TIME_DEFER_MODULE_ORDER_HTML: &str = "<!doctype html><html><head><script>window.deferLikeOrder=[];document.addEventListener('DOMContentLoaded',()=>{window.deferLikeOrder.push('dcl');window.deferLikeDclOrder=window.deferLikeOrder.join(',');});</script><script defer src=\"/assets/parse_time_defer_left.js\"></script><script type=\"module\">window.deferLikeOrder.push('module');Promise.resolve().then(()=>window.deferLikeOrder.push('module-microtask'));</script><script defer src=\"/assets/parse_time_defer_right.js\"></script></head><body><main id=\"late\">late</main></body></html>";
const PARSE_TIME_FINAL_CLASSIC_TERMINAL_BEFORE_DCL_HTML: &str = r#"<!doctype html>
<html><head>
<script>
window.finalClassicTerminalOrder = [];
document.addEventListener("DOMContentLoaded", () => {
  window.finalClassicTerminalOrder.push("dcl");
  window.finalClassicTerminalDclOrder = window.finalClassicTerminalOrder.join(",");
  window.finalClassicTerminalTimerAtDcl = window.finalClassicTerminalOrder.includes("timer");
});
</script>
<script defer src="/assets/parse_time_final_classic_terminal.js" onload="
  window.finalClassicTerminalOrder.push('script-load');
  setTimeout(() => window.finalClassicTerminalOrder.push('timer'), 0);
  Promise.resolve().then(() => window.finalClassicTerminalOrder.push('script-load-microtask'));
"></script>
</head><body><main>final classic terminal</main></body></html>"#;
const PARSE_TIME_FINAL_MODULE_TERMINAL_BEFORE_DCL_HTML: &str = r#"<!doctype html>
<html><head>
<script>
window.finalModuleTerminalOrder = [];
document.addEventListener("DOMContentLoaded", () => {
  window.finalModuleTerminalOrder.push("dcl");
  window.finalModuleTerminalDclOrder = window.finalModuleTerminalOrder.join(",");
  window.finalModuleTerminalTimerAtDcl = window.finalModuleTerminalOrder.includes("timer");
});
</script>
<script type="module" src="/assets/parse_time_final_module_terminal.mjs" onload="
  window.finalModuleTerminalOrder.push('module-load');
  setTimeout(() => window.finalModuleTerminalOrder.push('timer'), 0);
  Promise.resolve().then(() => window.finalModuleTerminalOrder.push('module-load-microtask'));
"></script>
</head><body><main>final module terminal</main></body></html>"#;
const PARSE_TIME_LIFECYCLE_TASKS_HTML: &str = "<!doctype html><html><head><script>window.lifecycleTaskOrder=[];document.addEventListener('DOMContentLoaded',()=>{window.lifecycleTaskOrder.push('dcl');window.lifecycleTaskDclOrder=window.lifecycleTaskOrder.join(',');window.lifecycleTaskDclSeen=true;Promise.resolve().then(()=>{window.lifecycleTaskOrder.push('dcl-microtask');window.lifecycleTaskDclMicrotaskSeen=true;window.lifecycleTaskDclAfterMicrotaskOrder=window.lifecycleTaskOrder.join(',');});});window.addEventListener('load',()=>{window.lifecycleTaskLoadSeen=true;window.lifecycleTaskOrder.push('load');window.lifecycleTaskFinalOrder=window.lifecycleTaskOrder.join(',');Promise.resolve().then(()=>{window.lifecycleTaskOrder.push('load-microtask');window.lifecycleTaskLoadMicrotaskSeen=true;window.lifecycleTaskAfterLoadMicrotaskOrder=window.lifecycleTaskOrder.join(',');});});</script><script defer src=\"/assets/parse_time_lifecycle_defer.js\"></script><script async src=\"/assets/parse_time_lifecycle_async.js\"></script></head><body><main id=\"late\">late</main></body></html>";
const ABORT_SIGNAL_ANY_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/abort_signal_any.html");
const BLOB_URLS_HTML: &str = include_str!("../../moli-core/tests/fixtures/runtime/blob_urls.html");
const DOM_RECT_HTML: &str = include_str!("../../moli-core/tests/fixtures/runtime/dom_rect.html");
const IMAGE_DATA_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/image_data.html");
const PARSER_IMAGE_FETCH_POLICY_HTML: &str = concat!(
    "<!doctype html><html><head>",
    "<link rel=\"stylesheet\" href=\"/assets/parser-image-fetch-policy.css?token={token}\">",
    "</head><body>",
    "<img id=\"parser-image\" src=\"/assets/parser-image-fetch-policy.svg?token={token}&source=html\">",
    "<div id=\"css-image\"></div>",
    "<main>parser image policy</main>",
    "</body></html>"
);
const DETACHED_EAGER_IMAGES_DELAY_LOAD_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/detached_eager_images_delay_load.html");
const LAZY_GEOMETRY_OFFSET_CHAIN_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/lazy_geometry_offset_chain.html");
const WEB_STREAMS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/web_streams.html");
const INTERSECTION_OBSERVER_OPTIONS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/intersection_observer_options.html");
const INTERSECTION_OBSERVER_ROOT_SCOPE_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/intersection_observer_root_scope.html");
const INTERSECTION_OBSERVER_ROOT_GEOMETRY_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/intersection_observer_root_geometry.html");
const INTERSECTION_OBSERVER_THRESHOLDS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/intersection_observer_thresholds.html");
const MUTATION_OBSERVER_OPTIONS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/mutation_observer_options.html");
const MUTATION_OBSERVER_ORDERING_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/mutation_observer_ordering.html");
const PERFORMANCE_MEASURE_OBSERVER_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/performance_measure_observer.html");
const MESSAGE_CHANNEL_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/message_channel.html");
const SHARED_WORKER_IFRAME_PERFORMANCE_OWNER_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/shared_worker_iframe_performance_owner.html"
);
const SHARED_WORKER_IFRAME_PERFORMANCE_OWNER_JS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/shared_worker_iframe_performance_owner.js"
);
const AUDIO_WORKLET_WASM_SOURCE_PHASE_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/audio_worklet_wasm_source_phase.html");
const MODULE_WASM_V128_GLOBAL_EXPORT_THROWS_TDZ_HTML: &str = r#"<!doctype html>
<html>
<body>
  <script type="module">
    window.moduleWasmV128TdzDone = false;
    try {
      const wasmExports = await import("/assets/mutable-global-with-v128.wasm");
      window.moduleWasmV128TdzMutableValue = wasmExports.mutableValue;
      try {
        void wasmExports.v128Export;
        window.moduleWasmV128TdzThrows = false;
      } catch (error) {
        window.moduleWasmV128TdzThrows = error instanceof ReferenceError;
        window.moduleWasmV128TdzErrorName = error && error.name;
      }
    } catch (error) {
      window.moduleWasmV128TdzError = String(error && (error.stack || error.message || error));
    }
    window.moduleWasmV128TdzDone = true;
  </script>
</body>
</html>"#;
const RANGE_BASIC_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/range_basic.html");
const RANGE_INTERNAL_ALGORITHMS_IGNORE_PAGE_TAMPERED_METHODS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/range_internal_algorithms_ignore_page_tampered_methods.html"
);
const SELECTION_BASIC_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/selection_basic.html");
const SELECTION_CONTAINS_NODE_IGNORES_PAGE_TAMPERED_NODE_CONTAINS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/selection_contains_node_ignores_page_tampered_node_contains.html"
);
const SELECTION_SET_BASE_AND_EXTENT_IGNORES_PAGE_TAMPERED_COMPARE_DOCUMENT_POSITION_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/selection_set_base_and_extent_ignores_page_tampered_compare_document_position.html"
);
const SELECTIONCHANGE_IGNORES_PAGE_TAMPERED_DOCUMENT_DISPATCH_EVENT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/selectionchange_ignores_page_tampered_document_dispatch_event.html"
);
const FORM_DATA_IGNORES_PAGE_TAMPERED_NODE_CONTAINS_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/form_data_ignores_page_tampered_node_contains.html"
);
const URL_FORM_DATA_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/url_form_data.html");
const SECONDARY_WEBAPIS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/secondary_webapis.html");
const BAIDU_BOOT_COMPAT_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><script src=\"/assets/baidu_boot_compat.js\"></script></head><body><main>boot</main></body></html>";
const BAIDU_LOCATION_REPLACE_BOOT_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><script src=\"/assets/baidu_location_replace_boot.js\"></script></head><body><main id=\"source\">boot-source</main></body></html>";
const BAIDU_BOOT_COMPAT_JS: &str = "(function(){const root=document.documentElement;for(let i=0;i<2000;i++){const d=new Date(1711267200000+i*1000);d.toLocaleString('en-US');d.toLocaleDateString('en-US');d.toLocaleTimeString('en-US');}root.setAttribute('data-mime-length', String(navigator.mimeTypes.length));root.setAttribute('data-plugin-length', String(navigator.plugins.length));root.setAttribute('data-pdf-viewer-enabled', String(navigator.pdfViewerEnabled));root.setAttribute('data-storage-instance', String(localStorage instanceof Storage && sessionStorage instanceof Storage));history.pushState({step:1}, '', '/compat/boot-pushed?from=push');history.replaceState({step:2}, '', '/compat/baidu-boot?boot=1');root.setAttribute('data-history-state', JSON.stringify(history.state));root.setAttribute('data-location', location.pathname + location.search);root.setAttribute('data-ok','1');})();";
const BAIDU_LOCATION_REPLACE_BOOT_JS: &str = "(function(){for(let i=0;i<256;i++){new Date(1711267200000+i*1000).toLocaleString('en-US');}if(navigator.mimeTypes.length===2&&navigator.plugins.length===5&&navigator.pdfViewerEnabled===true){location.replace('/location-nav/target?from=boot-script');}})();";
const PARSE_TIME_CLASSIC_JS: &str = "window.parseTimeExternal = 'external'; document.documentElement.setAttribute('data-external-before-late', document.getElementById('late') ? 'seen' : 'missing');";
const SCRIPT_SRC_BASE_ALPHA_JS: &str = "window.scriptSrcBaseResult = 'alpha';";
const SCRIPT_SRC_BASE_BETA_JS: &str = "window.scriptSrcBaseResult = 'beta';";
const PARSER_CONNECTED_LOAD_WRITE_TE_JS: &str = "document.write('te');";
const PARSE_TIME_DEFER_JS: &str = "window.parseTimeDeferSawTail = !!document.getElementById('late'); window.parseTimeDeferSawDcl = window.parseTimeDclFired === true;";
const PARSE_TIME_ASYNC_JS: &str = "window.parseTimeAsyncSawTail = !!document.getElementById('late'); window.parseTimeAsyncSawDcl = window.parseTimeDclFired === true;";
const PARSE_TIME_ASYNC_LOAD_ORDER_JS: &str = "window.parserOwnedAsyncLoadOrder.push('async-script');window.parserOwnedAsyncScriptSawTail=!!document.getElementById('late');window.parserOwnedAsyncScriptSawDcl=window.parserOwnedAsyncDclFired===true;";
const RUNTIME_OWNED_IN_ORDER_LOAD_JS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/runtime_owned_in_order_load.js");
const RUNTIME_OWNED_ASYNC_SLOW_JS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/runtime_owned_async_slow.js");
const RUNTIME_OWNED_ASYNC_FAST_JS: &str =
    "window.runtimeOwnedAsyncClassicOrder.push('external-script:' + document.readyState);";
const RUNTIME_OWNED_DEFAULT_ASYNC_MODULE_SIDE_EFFECT_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/runtime_owned_default_async_module_side_effect.mjs"
);
const PARSE_TIME_ASYNC_SLOW_JS: &str = "window.parseTimeAsyncSlowSawTail = !!document.getElementById('late'); window.parseTimeAsyncSlowSawDcl = window.parseTimeDclFired === true;";
const PARSE_TIME_ASYNC_TASK_FIRST_JS: &str = "window.asyncTaskOrder.push('first-script'); Promise.resolve().then(() => window.asyncTaskOrder.push('first-microtask'));";
const PARSE_TIME_ASYNC_TASK_SECOND_JS: &str = "window.asyncTaskOrder.push('second-script'); window.asyncTaskOrderResult = window.asyncTaskOrder.join(',');";
const PARSE_TIME_ASYNC_TASK_VISIBILITY_FIRST_JS: &str = "window.asyncTaskVisibilityOrder.push('first-script'); window.asyncTaskVisibilityFirstSawTail = !!document.getElementById('late'); window.asyncTaskVisibilityFirstSawDcl = window.parseTimeVisibilityDclFired === true; Promise.resolve().then(() => window.asyncTaskVisibilityOrder.push('first-microtask'));";
const PARSE_TIME_ASYNC_TASK_VISIBILITY_SECOND_JS: &str = "window.asyncTaskVisibilityOrder.push('second-script'); window.asyncTaskVisibilityOrderResult = window.asyncTaskVisibilityOrder.join(',');";
const PARSE_TIME_ASYNC_POST_PARSE_FIRST_JS: &str = "window.postParseAsyncTaskOrder.push('first-script'); Promise.resolve().then(() => window.postParseAsyncTaskOrder.push('first-microtask'));";
const PARSE_TIME_ASYNC_POST_PARSE_SECOND_JS: &str = "window.postParseAsyncTaskOrder.push('second-script'); window.postParseAsyncTaskOrder.push(window.postParseDclSeen ? 'second-after-dcl' : 'second-before-dcl');";
const PARSE_TIME_ASYNC_POST_PARSE_SLOW_FIRST_JS: &str = "window.postParseSlowTaskOrder.push('first-script'); Promise.resolve().then(() => window.postParseSlowTaskOrder.push('first-microtask'));";
const PARSE_TIME_ASYNC_POST_PARSE_SLOW_SECOND_JS: &str = "window.postParseSlowTaskOrder.push('second-script'); window.postParseSlowTaskOrder.push(window.postParseSlowDclSeen ? 'second-after-dcl' : 'second-before-dcl'); window.postParseSlowFinalOrder = window.postParseSlowTaskOrder.join(',');";
const PARSE_TIME_ASYNC_SLOW_CHUNKED_FIRST_JS: &str = "window.slowChunkOrder.push('slow-async'); window.slowChunkFinalOrder = window.slowChunkOrder.join(',');";
const PARSE_TIME_ASYNC_SLOW_CHUNKED_DEFER_JS: &str = "window.slowChunkOrder.push('defer');";
const PARSE_TIME_DEFER_LEFT_JS: &str = "window.deferLikeOrder.push('defer-left'); Promise.resolve().then(() => window.deferLikeOrder.push('defer-left-microtask'));";
const PARSE_TIME_DEFER_RIGHT_JS: &str = "window.deferLikeOrder.push('defer-right'); Promise.resolve().then(() => window.deferLikeOrder.push('defer-right-microtask'));";
const PARSE_TIME_FINAL_CLASSIC_TERMINAL_JS: &str = "window.finalClassicTerminalOrder.push('script-body'); Promise.resolve().then(() => window.finalClassicTerminalOrder.push('script-body-microtask'));";
const PARSE_TIME_FINAL_MODULE_TERMINAL_JS: &str = "window.finalModuleTerminalOrder.push('module-body'); Promise.resolve().then(() => window.finalModuleTerminalOrder.push('module-body-microtask'));";
const PARSE_TIME_LIFECYCLE_DEFER_JS: &str = "window.lifecycleTaskOrder.push('defer-script'); Promise.resolve().then(() => window.lifecycleTaskOrder.push('defer-microtask'));";
const PARSE_TIME_LIFECYCLE_ASYNC_JS: &str =
    "window.lifecycleTaskAsyncSeen=true; window.lifecycleTaskOrder.push('async-script');";
const PARSE_TIME_DYNAMIC_CLOBBER_JS: &str = "window.parseTimeDynamicLoadOrder.push('clobber-script');window.page={};const script=document.createElement('script');script.src='/assets/parse_time_dynamic_followup.js';script.onload=()=>{window.parseTimeDynamicLoadOrder.push('dynamic-load');window.parseTimeDynamicSaw=window.page&&window.page.comm&&window.page.comm.invokeApps?window.page.comm.invokeApps.marker:'missing';window.parseTimeDynamicUnsafeMarker=window.page.comm.invokeApps.marker;window.parseTimeDynamicFinalOrder=window.parseTimeDynamicLoadOrder.join(',');};document.head.appendChild(script);";
const PARSE_TIME_DYNAMIC_FOLLOWUP_JS: &str =
    "window.parseTimeDynamicLoadOrder.push('dynamic-script');";
const PARSE_TIME_DYNAMIC_ERROR_CLOBBER_JS: &str = "window.parseTimeDynamicErrorOrder.push('clobber-script');window.page={};const script=document.createElement('script');script.src='/assets/parse_time_dynamic_missing.js';script.onerror=()=>{window.parseTimeDynamicErrorOrder.push('dynamic-error');window.parseTimeDynamicErrorSaw=window.page&&window.page.comm&&window.page.comm.invokeApps?window.page.comm.invokeApps.marker:'missing';window.parseTimeDynamicErrorFinalOrder=window.parseTimeDynamicErrorOrder.join(',');};document.head.appendChild(script);";
const DOCUMENT_WRITE_PAGE_TASK_CLOBBER_JS: &str =
    "window.documentWritePageTaskOrder.push('written-script');window.page={};";
const DOCUMENT_WRITE_DELAYED_EXTERNAL_JS: &str = "window.documentWriteDelayedOrder.push('external-script'); window.documentWriteDelayedExternalRan = true; window.documentWriteDelayedExternalSawOuter = window.documentWriteDelayedOrder.includes('outer-after-write');";
const DOCUMENT_OPEN_PARSER_EXTERNAL_JS: &str = "window.documentOpenParserExternalRan = true; window.documentOpenParserOrder.push('external-script');";
const DOCUMENT_WRITE_NESTED_EXTERNAL_PARENT_JS: &str = r#"window.documentWriteNestedExternalOrder.push("parent");document.write('<script src="/assets/document_write_nested_external_');document.write('child.js"><\/script>');window.documentWriteNestedExternalOrder.push("parent-after-write");"#;
const DOCUMENT_WRITE_NESTED_EXTERNAL_CHILD_JS: &str =
    r#"window.documentWriteNestedExternalOrder.push("child");"#;
const DOCUMENT_WRITE_NESTED_EXTERNAL_OUTER_AFTER_JS: &str =
    r#"window.documentWriteNestedExternalOrder.push("outer-after");"#;
const DOCUMENT_WRITE_EXTERNAL_SPLIT_SESSION_PARENT_JS: &str = r#"window.documentWriteExternalSplitSessionOrder.push("parent");document.write('<script>window.documentWriteExternalSplitSessionOrder.push("written-inline");window.documentWriteExternalSplitSessionSawTail=!!document.getElementById("tail")');document.write(';window.documentWriteExternalSplitSessionOrder.push(window.documentWriteExternalSplitSessionSawTail?"tail-visible":"tail-hidden")<\/script>');window.documentWriteExternalSplitSessionOrder.push("parent-after-write");"#;
const DOCUMENT_WRITE_INSERTED_CHUNKED_EXTERNAL_JS: &str =
    r#"window.documentWriteInsertedChunkedOrder.push("inserted-external");"#;
const BLOCKING_STYLESHEET_PARSER_BLOCKING_JS: &str = "window.blockingStylesheetParserElapsed = Date.now() - window.blockingStylesheetStart; window.blockingStylesheetParserBlockedEnough = window.blockingStylesheetParserElapsed >= 50; window.blockingStylesheetParserSawLate = !!document.getElementById('late'); window.blockingStylesheetParserSawDcl = window.blockingStylesheetDclSeen === true; window.blockingStylesheetOrder.push('parser');";
const BLOCKING_STYLESHEET_PARSER_BLOCKING_DOCUMENT_WRITE_JS: &str = "window.blockingStylesheetParserWriteRan = true; window.blockingStylesheetParserWriteElapsed = Date.now() - window.blockingStylesheetStart; window.blockingStylesheetParserWriteBlockedEnough = window.blockingStylesheetParserWriteElapsed >= 50; window.blockingStylesheetParserWriteSawLate = !!document.getElementById('late'); document.write('<span id=\"written-before-late\">written</span>'); window.blockingStylesheetParserWriteSawWrittenDuringScript = !!document.getElementById('written-before-late'); window.blockingStylesheetOrder.push('parser-write');";
const BLOCKING_STYLESHEET_DEFER_JS: &str = "window.blockingStylesheetDeferElapsed = Date.now() - window.blockingStylesheetStart; window.blockingStylesheetDeferBlockedEnough = window.blockingStylesheetDeferElapsed >= 50; window.blockingStylesheetDeferSawLate = !!document.getElementById('late'); window.blockingStylesheetDeferSawDcl = window.blockingStylesheetDeferOrder.includes('dcl'); window.blockingStylesheetDeferOrder.push('defer');";
const PHASE_TWO_UPGRADE_RUNTIME_STYLE_DEFER_JS: &str = "window.phaseTwoUpgradeDeferSawLoaded = window.phaseTwoUpgradeStyleLoaded === true; window.phaseTwoUpgradeDeferSawLate = !!document.getElementById('late'); window.phaseTwoUpgradeStyleOrder.push('defer'); Promise.resolve().then(() => { window.phaseTwoUpgradeStyleOrder.push('defer-microtask'); window.phaseTwoUpgradeStyleFinalOrder = window.phaseTwoUpgradeStyleOrder.join(','); }); window.phaseTwoUpgradeStyleFinalOrder = window.phaseTwoUpgradeStyleOrder.join(',');";
const BLOCKING_STYLESHEET_ALTERNATE_PROBE_JS: &str = "window.blockingStylesheetAlternateElapsed = Date.now() - window.blockingStylesheetAlternateStart; window.blockingStylesheetAlternateBlocked = window.blockingStylesheetAlternateElapsed >= 50; window.blockingStylesheetAlternateSawLate = !!document.getElementById('late'); window.blockingStylesheetAlternateSawDcl = window.blockingStylesheetAlternateDclSeen === true;";
const DYNAMIC_BLOCKING_STYLESHEET_RUNTIME_JS: &str = "window.dynamicBlockingStylesheetSawStyleLoaded = window.dynamicBlockingStylesheetStyleLoaded === true; window.dynamicBlockingStylesheetSawLate = !!document.getElementById('late'); window.dynamicBlockingStylesheetSawDcl = window.dynamicBlockingStylesheetOrder.includes('dcl'); window.dynamicBlockingStylesheetSawLoad = window.dynamicBlockingStylesheetOrder.includes('load'); window.dynamicBlockingStylesheetOrder.push('dynamic'); window.dynamicBlockingStylesheetFinalOrder = window.dynamicBlockingStylesheetOrder.join(','); fetch('/assets/dynamic_blocking_stylesheet_script_executed', { cache: 'no-store' });";
const RUNTIME_CONNECTED_MODULEPRELOAD_SLOW_MJS: &str = "export const slow = 'modulepreload-ok';";
const MODULEPRELOAD_SHARED_ROOT_MJS: &str = r#"import { seenLeafOrder, value } from "./modulepreload_shared_mid.mjs";
window.modulepreloadSharedOrder.push("root");
window.modulepreloadSharedValue = value;
window.modulepreloadSharedSawLeafBeforeMid = seenLeafOrder.includes("leaf");
window.modulepreloadSharedFinalOrder = window.modulepreloadSharedOrder.join(",");"#;
const MODULEPRELOAD_SHARED_MID_MJS: &str = r#"import { value } from "./modulepreload_shared_leaf_slow.mjs";
window.modulepreloadSharedOrder.push("mid");
export { value };
export const seenLeafOrder = window.modulepreloadSharedOrder.join(",");"#;
const MODULEPRELOAD_SHARED_LEAF_SLOW_MJS: &str = r#"window.modulepreloadSharedOrder.push("leaf");
export const value = "leaf-ok";"#;
const MODULEPRELOAD_DUPLICATE_ROOT_MJS: &str = r#"import { value as a } from "./modulepreload_duplicate_parent_a.mjs";
import { value as b } from "./modulepreload_duplicate_parent_b.mjs";
window.modulepreloadDuplicateOrder.push("root");
window.modulepreloadDuplicateValue = `${a}|${b}`;
window.modulepreloadDuplicateFinalOrder = window.modulepreloadDuplicateOrder.join(",");"#;
const MODULEPRELOAD_DUPLICATE_PARENT_A_MJS: &str = r#"import { value } from "./modulepreload_duplicate_leaf_slow.mjs";
window.modulepreloadDuplicateOrder.push("parent-a");
export { value };"#;
const MODULEPRELOAD_DUPLICATE_PARENT_B_MJS: &str = r#"import { value } from "./modulepreload_duplicate_leaf_slow.mjs";
window.modulepreloadDuplicateOrder.push("parent-b");
export { value };"#;
const MODULEPRELOAD_DUPLICATE_LEAF_SLOW_MJS: &str = r#"window.modulepreloadDuplicateLeafEvalCount = (window.modulepreloadDuplicateLeafEvalCount || 0) + 1;
window.modulepreloadDuplicateOrder.push("leaf");
export const value = "shared-ok";"#;
const DUPLICATE_MODULE_ROOT_EVAL_MJS: &str = r#"window.duplicateModuleRootEvalCount = (window.duplicateModuleRootEvalCount || 0) + 1;
window.duplicateModuleRootOrder.push("root");
Promise.resolve().then(() => {
  window.duplicateModuleRootOrder.push("root-microtask");
  window.duplicateModuleRootFinalOrder = window.duplicateModuleRootOrder.join(",");
});
export const value = "root-ok";"#;
const DUPLICATE_NESTED_THIS_MJS: &str = r#"window.duplicateNestedModuleLog.push(typeof this === "undefined" ? "this-undefined" : "this-defined");"#;
const DUPLICATE_NESTED_THIS_NESTED_MJS: &str = r#"import "./duplicate_nested_this.mjs";
window.duplicateNestedModuleLog.push("this-nested");"#;
const MODULE_WRONG_MIME_CSS: &str = "#wrong-mime-module { color: red; }";
const MODULEPRELOAD_REUSED_ROOT_SLOW_MJS: &str = r#"import { sawChildBeforeParent, value } from "./modulepreload_reused_parent.mjs";
window.modulepreloadReusedParentOrder.push("root");
window.modulepreloadReusedParentValue = value;
window.modulepreloadReusedParentSawChildBeforeParent = sawChildBeforeParent;
window.modulepreloadReusedParentFinalOrder = window.modulepreloadReusedParentOrder.join(",");"#;
const MODULEPRELOAD_REUSED_PARENT_MJS: &str = r#"import { value } from "./modulepreload_reused_child_slow.mjs";
window.modulepreloadReusedParentOrder.push("parent");
export { value };
export const sawChildBeforeParent = window.modulepreloadReusedParentOrder.includes("child");"#;
const MODULEPRELOAD_REUSED_CHILD_SLOW_MJS: &str = r#"window.modulepreloadReusedParentOrder.push("child");
export const value = "child-ok";"#;
const SHADOW_ADOPTED_MODULEPRELOAD_CSS: &str = "span { color: blue }";
const DYNAMIC_TAXONOMY_ASYNC_FAST_JS: &str = "window.dynamicScriptTaxonomyOrder.push('async-fast'); window.dynamicScriptTaxonomyOrderResult = window.dynamicScriptTaxonomyOrder.join(',');";
const DYNAMIC_TAXONOMY_IN_ORDER_SLOW_JS: &str = "if (window.dynamicScriptTaxonomyOrder) { window.dynamicScriptTaxonomyOrder.push('in-order-slow'); window.dynamicScriptTaxonomyOrderResult = window.dynamicScriptTaxonomyOrder.join(','); } if (window.dynamicInOrderScriptOrder) { window.dynamicInOrderScriptOrder.push('in-order-slow'); window.dynamicInOrderScriptOrderResult = window.dynamicInOrderScriptOrder.join(','); }";
const DYNAMIC_TAXONOMY_IN_ORDER_FAST_JS: &str = "window.dynamicInOrderScriptOrder.push('in-order-fast'); window.dynamicInOrderScriptOrderResult = window.dynamicInOrderScriptOrder.join(',');";
const DYNAMIC_PREPARATION_CONTEXT_STALE_JS: &str = "window.staleDynamicRan = true; window.dynamicPreparationContextOrder = window.dynamicPreparationContextOrder || []; window.dynamicPreparationContextOrder.push('stale'); window.dynamicPreparationContextResult = window.dynamicPreparationContextOrder.join(',');";
const DYNAMIC_PREPARATION_CONTEXT_OPEN_JS: &str = r#"document.open(); document.write(`<!doctype html><html><body><script>window.documentOpenAsyncRan = true; window.dynamicPreparationContextOrder = ['replacement']; window.dynamicPreparationContextResult = 'pending'; setTimeout(() => { window.dynamicPreparationContextResult = window.staleDynamicRan ? 'stale-ran' : 'replacement-only'; }, 0);</script></body></html>`); document.close();"#;
const DOCUMENT_WRITE_IMPLICIT_REPLACE_ASYNC_JS: &str = r#"window.documentWriteImplicitOrder.push('dynamic'); document.write(`<!doctype html><html><body><main id="new">new</main><script>window.documentWriteImplicitOrder.push('replacement'); window.documentWriteImplicitReplaceRan = true; window.documentWriteImplicitResult = 'pending'; window.documentWriteImplicitOrderResult = window.documentWriteImplicitOrder.join(','); setTimeout(() => { window.documentWriteImplicitResult = window.documentWriteImplicitStaleDeferRan ? 'stale-ran' : 'replacement-only'; window.documentWriteImplicitOrderResult = window.documentWriteImplicitOrder.join(','); }, 0);</script></body></html>`);"#;
const DOCUMENT_WRITE_IMPLICIT_REPLACE_ASYNC_MODULE_JS: &str = r#"window.documentWriteImplicitModuleOrder.push('dynamic'); document.write(`<!doctype html><html><body><main id="new">new</main><script>window.documentWriteImplicitModuleOrder.push('replacement'); window.documentWriteImplicitModuleReplaceRan = true; window.documentWriteImplicitModuleResult = 'pending'; window.documentWriteImplicitModuleOrderResult = window.documentWriteImplicitModuleOrder.join(','); setTimeout(() => { window.documentWriteImplicitModuleResult = window.documentWriteImplicitStaleModuleRan ? 'stale-ran' : 'replacement-only'; window.documentWriteImplicitModuleOrderResult = window.documentWriteImplicitModuleOrder.join(','); }, 0);</script></body></html>`);"#;
const DOCUMENT_WRITE_IMPLICIT_REPLACE_STALE_DEFER_JS: &str = "window.documentWriteImplicitOrder.push('stale-defer'); window.documentWriteImplicitStaleDeferRan = true; window.documentWriteImplicitOrderResult = window.documentWriteImplicitOrder.join(',');";
const DOCUMENT_WRITE_IMPLICIT_REPLACE_STALE_MODULE_MJS: &str = "window.documentWriteImplicitModuleOrder.push('stale-module'); window.documentWriteImplicitStaleModuleRan = true; window.documentWriteImplicitModuleOrderResult = window.documentWriteImplicitModuleOrder.join(','); export {};";
const DOCUMENT_WRITE_REPLACEMENT_ASYNC_BOOT_JS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/document_write_replacement_async_boot.js");
const DOCUMENT_WRITE_REPLACEMENT_ASYNC_JS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/document_write_replacement_async.js");
const DOCUMENT_WRITE_EXTERNAL_PARSER_BLOCKING_JS: &str = "window.documentWriteExternalScriptRan = true; window.documentWriteExternalSawBefore = !!document.getElementById('before-written'); window.documentWriteExternalSawAfter = !!document.getElementById('after-written');";
const DOCUMENT_WRITE_LOAD_MICROTASK_JS: &str =
    "window.documentWriteLoadMicrotaskOrder.push('external-script');";
const DOCUMENT_WRITE_DEFER_WRITTEN_JS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/document_write_defer_written.js");
const DOCUMENT_WRITE_DEFER_WRITTEN_DCL_JS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/document_write_defer_written_dcl.js");
const DOCUMENT_OPEN_AFTER_LOAD_EXTERNAL_1_JS: &str =
    "window.documentOpenAfterLoadOrder.push('external-1');";
const DOCUMENT_OPEN_AFTER_LOAD_EXTERNAL_2_JS: &str = "window.documentOpenAfterLoadOrder.push('external-2'); window.documentOpenAfterLoadResult = window.documentOpenAfterLoadOrder.join(',');";
const DOCUMENT_WRITE_IMPORTMAP_WRITTEN_MODULE_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/document_write_importmap_written_module.mjs"
);
const BLOCKING_STYLESHEET_SLOW_CSS: &str = "body { color: rgb(1, 2, 3); }";
const URL_BINDING_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/url_binding.html");
const CHROME_STYLESHEETLIST_1_CSS: &str = "#e1 { color: red; }";
const CHROME_STYLESHEETLIST_2_CSS: &str = "#e2 { color: red; }";
const CHROME_STYLESHEETLIST_3_CSS: &str = "#e3 { color: red; }";
const SELECTOR_CORNER_CASES_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/selector_corner_cases.html");
const SELECTOR_HOST_BRIDGE_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/selector_host_bridge.html");
const NATIVE_BRIDGE_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/native_bridge.html");
const EVENT_COLLECTIONS_BRIDGE_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/event_collections_bridge.html");
const LIFECYCLE_BRIDGE_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/lifecycle_bridge.html");
const MAIN_DOCUMENT_LIFECYCLE_PERFORMANCE_EVENT_END_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/main_document_lifecycle_performance_event_end.html"
);
const LIVE_COLLECTIONS_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/live_collections.html");
const TREE_BRIDGE_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/tree_bridge.html");
const RUST_DOM_SOURCE_OF_TRUTH_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/rust_dom_source_of_truth.html");
const RUST_DOM_LAZY_HYDRATION_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/rust_dom_lazy_hydration.html");
const RUST_DOM_MUTATION_SYNC_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/rust_dom_mutation_sync.html");
const RUST_DOM_DOCUMENT_OPEN_SYNC_HTML: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/rust_dom_document_open_sync.html");
const RUST_DOM_FRAGMENT_SCRIPT_SYNC_HTML: &str = "<!doctype html><html><body><script>const fragment=document.createDocumentFragment();const script=document.createElement('script');script.text='window.fragmentScriptRuns=(window.fragmentScriptRuns||0)+1;';fragment.appendChild(script);document.body.appendChild(fragment);window.fragmentScriptConnected=script.isConnected;window.fragmentScriptParentIsBody=script.parentNode===document.body;window.fragmentFragmentEmpty=fragment.childNodes.length===0;</script></body></html>";
const APP_JS: &str = "window.__fixtureReady = true;";
const SEQUENCE_JS: &str =
    "window.executionOrder.push('external-normal'); window.externalReady = true;";
const IMPORTMAP_SCOPED_IMPORTED_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/importmap_scoped_imported.mjs");
const IMPORTMAP_INITIAL_TARGET_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/importmap_initial_target.mjs");
const IMPORTMAP_OVERRIDE_TARGET_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/importmap_override_target.mjs");
const IMPORTMAP_EXTRA_TARGET_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/importmap_extra_target.mjs");
const IMPORTMAP_CANONICAL_TARGET_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/importmap_canonical_target.mjs");
const MODULE_TLA_DEPENDENCY_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_tla_dependency.mjs");
const MODULE_DEFAULT_EXPORT_VALUE_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_default_export_value.mjs");
const MODULE_DEFAULT_REEXPORT_SOURCE_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_default_reexport_source.mjs");
const MODULE_DEFAULT_REEXPORT_BARREL_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_default_reexport_barrel.mjs");
const MODULE_STRING_LITERAL_EXPORT_NAMES_SOURCE_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_string_literal_export_names_source.mjs"
);
const MODULE_STRING_LITERAL_EXPORT_NAMES_BARREL_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_string_literal_export_names_barrel.mjs"
);
const MODULE_STRING_LITERAL_EXPORT_NAMES_SURROGATE_PAIRS_SOURCE_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_string_literal_export_names_surrogate_pairs_source.mjs"
);
const MODULE_STRING_LITERAL_EXPORT_NAMES_SURROGATE_PAIRS_BARREL_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_string_literal_export_names_surrogate_pairs_barrel.mjs"
);
const MODULE_EXPORT_STAR_STRING_LITERAL_NAMESPACE_BARREL_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_export_star_string_literal_namespace_barrel.mjs"
);
const MODULE_ESCAPED_IDENTIFIER_NAMES_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_escaped_identifier_names.mjs");
const MODULE_MULTILINE_SOURCE_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_multiline_source.mjs");
const MODULE_MULTILINE_BARREL_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_multiline_barrel.mjs");
const MODULE_IMPORT_ASSERTIONS_LEGACY_BARREL_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module-import-assertions-legacy-barrel.mjs"
);
const MODULE_EXPORT_STAR_SOURCE_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_export_star_source.mjs");
const MODULE_EXPORT_STAR_BARREL_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_export_star_barrel.mjs");
const MODULE_EXPORT_STAR_AMBIGUOUS_A_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_export_star_ambiguous_a.mjs");
const MODULE_EXPORT_STAR_AMBIGUOUS_B_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_export_star_ambiguous_b.mjs");
const MODULE_EXPORT_STAR_AMBIGUOUS_BARREL_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_export_star_ambiguous_barrel.mjs");
const MODULE_CYCLE_MISSING_EXPORT_A_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_cycle_missing_export_a.mjs");
const MODULE_CYCLE_MISSING_EXPORT_B_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_cycle_missing_export_b.mjs");
const MODULE_CYCLE_INITIALIZING_MISSING_EXPORT_A_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_initializing_missing_export_a.mjs"
);
const MODULE_CYCLE_INITIALIZING_MISSING_EXPORT_B_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_initializing_missing_export_b.mjs"
);
const MODULE_CYCLE_DEFAULT_MISSING_A_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_cycle_default-missing-a.mjs");
const MODULE_CYCLE_DEFAULT_MISSING_B_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_cycle_default-missing-b.mjs");
const MODULE_CYCLE_DYNAMIC_IMPORT_WAITS_A_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_cycle_dynamic_import_waits_a.mjs");
const MODULE_CYCLE_DYNAMIC_IMPORT_WAITS_B_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_cycle_dynamic_import_waits_b.mjs");
const MODULE_CYCLE_EXPORT_STAR_LATE_BARREL_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_cycle_export_star_late_barrel.mjs");
const MODULE_CYCLE_EXPORT_STAR_LATE_SOURCE_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_cycle_export_star_late_source.mjs");
const MODULE_CYCLE_EXPORT_STAR_MULTIHOP_OUTER_BARREL_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_multihop_outer_barrel.mjs"
);
const MODULE_CYCLE_EXPORT_STAR_MULTIHOP_INNER_BARREL_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_multihop_inner_barrel.mjs"
);
const MODULE_CYCLE_EXPORT_STAR_MULTIHOP_SOURCE_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_multihop_source.mjs"
);
const MODULE_CYCLE_EXPORT_STAR_LATE_AMBIGUOUS_BARREL_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_late_ambiguous_barrel.mjs"
);
const MODULE_CYCLE_EXPORT_STAR_LATE_AMBIGUOUS_A_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_late_ambiguous_a.mjs"
);
const MODULE_CYCLE_EXPORT_STAR_LATE_AMBIGUOUS_B_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_late_ambiguous_b.mjs"
);
const MODULE_CYCLE_EXPORT_STAR_MULTIHOP_LATE_AMBIGUOUS_OUTER_BARREL_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_multihop_late_ambiguous_outer_barrel.mjs"
);
const MODULE_CYCLE_EXPORT_STAR_MULTIHOP_LATE_AMBIGUOUS_INNER_BARREL_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_multihop_late_ambiguous_inner_barrel.mjs"
);
const MODULE_CYCLE_EXPORT_STAR_MULTIHOP_LATE_AMBIGUOUS_A_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_multihop_late_ambiguous_a.mjs"
);
const MODULE_CYCLE_EXPORT_STAR_MULTIHOP_LATE_AMBIGUOUS_B_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_cycle_export_star_multihop_late_ambiguous_b.mjs"
);
const MODULE_PENDING_STAR_CYCLE_ENTRY_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_pending_star_cycle_entry.mjs");
const MODULE_PENDING_STAR_CYCLE_A_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_pending_star_cycle_a.mjs");
const MODULE_PENDING_STAR_CYCLE_B_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_pending_star_cycle_b.mjs");
const MODULE_PENDING_STAR_CYCLE_C_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_pending_star_cycle_c.mjs");
const MODULE_PENDING_STAR_BODY_CYCLE_A_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_pending_star_body_cycle_a.mjs");
const MODULE_PENDING_STAR_BODY_CYCLE_B_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_pending_star_body_cycle_b.mjs");
const MODULE_SHARED_INITIALIZING_DEP_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_shared_initializing_dep.mjs");
const MODULE_SHARED_INITIALIZING_PARENT_A_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_shared_initializing_parent_a.mjs");
const MODULE_SHARED_INITIALIZING_PARENT_B_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_shared_initializing_parent_b.mjs");
const MODULE_SHARED_FAILED_DEP_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_shared_failed_dep.mjs");
const MODULE_SHARED_FAILED_PARENT_A_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_shared_failed_parent_a.mjs");
const MODULE_SHARED_FAILED_PARENT_B_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_shared_failed_parent_b.mjs");
const MODULE_SHARED_UNSUPPORTED_DEP_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_shared_unsupported_dep.mjs");
const MODULE_SHARED_UNSUPPORTED_PARENT_A_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_shared_unsupported_parent_a.mjs");
const MODULE_SHARED_UNSUPPORTED_PARENT_B_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_shared_unsupported_parent_b.mjs");
const MODULE_LINK_EXPORTS_ONLY_NAMED_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_link_exports_only_named.mjs");
const MODULE_SIDE_EFFECT_ONLY_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_side_effect_only.mjs");
const MODULE_DEFAULT_FUNCTION_EXPORT_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_default_function_export.mjs");
const MODULE_DEFAULT_CLASS_EXPORT_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_default_class_export.mjs");
const MODULE_DEFAULT_ANONYMOUS_FUNCTION_EXPORT_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_default_anonymous_function_export.mjs"
);
const MODULE_DEFAULT_ANONYMOUS_CLASS_EXPORT_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_default_anonymous_class_export.mjs"
);
const MODULE_EXPORT_CLASS_NAMED_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_export_class_named.mjs");
const MODULE_EXPORT_GENERATOR_FUNCTIONS_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_export_generator_functions.mjs");
const MODULE_EXPORT_CONST_MULTIPLE_BINDINGS_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_export_const_multiple_bindings.mjs"
);
const MODULE_EXPORT_DESTRUCTURING_BINDINGS_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_export_destructuring_bindings.mjs");
const MODULE_EXPORT_NESTED_DESTRUCTURING_BINDINGS_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_export_nested_destructuring_bindings.mjs"
);
const MODULE_EXPORT_NESTED_INITIALIZER_COMMAS_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_export_nested_initializer_commas.mjs"
);
const MODULE_IMPORT_EXPORT_LIST_COMMENTS_SOURCE_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_import_export_list_comments_source.mjs"
);
const MODULE_IMPORT_EXPORT_LIST_COMMENTS_BARREL_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_import_export_list_comments_barrel.mjs"
);
const MODULE_MULTILINE_DYNAMIC_IMPORT_TARGET_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_multiline_dynamic_import_target.mjs"
);
const MODULE_DYNAMIC_IMPORT_COMMENTS_TARGET_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_dynamic_import_comments_target.mjs"
);
const MODULE_DYNAMIC_IMPORT_TEMPLATE_TARGET_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_dynamic_import_template_target.mjs"
);
const MODULE_ESCAPED_STRING_LITERAL_SPECIFIERS_SOURCE_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_escaped_string_literal_specifiers_source.mjs"
);
const MODULE_ESCAPED_STRING_LITERAL_SPECIFIERS_BARREL_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_escaped_string_literal_specifiers_barrel.mjs"
);
const MODULE_EXPORT_VARIABLE_LIVE_BINDINGS_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_export_variable_live_bindings.mjs");
const MODULE_SELF_BARE_DYNAMIC_IMPORT_RESOLVES_AFTER_OWN_EVALUATION_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_self_bare_dynamic_import_resolves_after_own_evaluation.mjs"
);
const MODULE_SELF_BARE_DYNAMIC_IMPORT_AFTER_SETTLE_RESOLVES_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_self_bare_dynamic_import_after_settle_resolves.mjs"
);
const MODULE_RUNTIME_HELPER_SHADOWING_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_runtime_helper_shadowing.mjs");
const MODULE_RUNTIME_HELPER_SHADOWING_SOURCE_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_runtime_helper_shadowing_source.mjs"
);
const DYNAMIC_ASYNC_MODULE_ACQUISITION_BARRIER_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/dynamic_async_module_acquisition_barrier.mjs"
);
const MODULE_PKG_ENTRY_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_pkg_entry.mjs");
const MODULE_PKG_SCOPED_ENTRY_MJS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/module_pkg_scoped_entry.mjs");
const MODULE_TOP_LEVEL_AWAIT_DELAYS_DOMCONTENTLOADED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_top_level_await_delays_domcontentloaded.html"
);
const MODULE_TOP_LEVEL_AWAIT_OVER_FIFTY_MS_DELAYS_DOMCONTENTLOADED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_top_level_await_over_fifty_ms_delays_domcontentloaded.html"
);
const MODULE_TLA_DEPENDENCY_DELAYS_PARENT_AND_DOMCONTENTLOADED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_tla_dependency_delays_parent_and_domcontentloaded.html"
);
const PARSER_OWNED_MODULE_TLA_DYNAMIC_IMPORT_DELAYS_DOMCONTENTLOADED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_owned_module_tla_dynamic_import_delays_domcontentloaded.html"
);
const PARSER_OWNED_MODULE_TLA_DYNAMIC_IMPORT_DEP_MJS: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/parser_owned_module_tla_dynamic_import_dep.mjs"
);
const STYLESHEET_MEDIA_CHANGE_LOAD_HANDLER_DOES_NOT_REQUEUE_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/stylesheet_media_change_load_handler_does_not_requeue.html"
);
const STYLESHEET_MEDIA_CHANGE_LOAD_HANDLER_CSS: &str =
    include_str!("../../moli-core/tests/fixtures/runtime/stylesheet_media_change_load_handler.css");
const MODULE_SHARED_FAILED_DEPENDENCY_IS_NOT_REEXECUTED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_shared_failed_dependency_is_not_reexecuted.html"
);
const MODULE_SHARED_UNSUPPORTED_DEPENDENCY_IS_NOT_RETRIED_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_shared_unsupported_dependency_is_not_retried.html"
);
const MODULE_EXPORT_STAR_AND_NAMESPACE_REEXPORT_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_export_star_and_namespace_reexport.html"
);
const MODULE_MULTILINE_IMPORT_AND_EXPORT_LIST_HTML: &str = include_str!(
    "../../moli-core/tests/fixtures/runtime/module_multiline_import_and_export_list.html"
);
pub fn next_parser_image_fetch_policy_token() -> String {
    routes_core::next_parser_image_fetch_policy_token()
}

pub fn parser_image_fetch_policy_asset_request_count(token: &str, source: &str) -> usize {
    routes_core::parser_image_fetch_policy_asset_request_count(token, source)
}

pub fn scrapling_dynamic_fetcher_smoke_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/scrapling_dynamic_fetcher_smoke.py")
}

pub use server::FixtureServer;
