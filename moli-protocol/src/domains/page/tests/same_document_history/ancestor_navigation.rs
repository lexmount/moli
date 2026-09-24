use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const PROBE: &str = include_str!("ancestor_navigation.js");
const PARENT: &str = r#"<!doctype html><body><script>
const params = new URLSearchParams(location.search);
const frame = document.createElement('iframe');
frame.id = 'source';
if (params.has('sandbox')) frame.setAttribute('sandbox', params.get('sandbox'));
frame.src = params.get('source');
document.body.append(frame);
</script>"#;

struct RemoteOrigin {
    base_url: String,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for RemoteOrigin {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl RemoteOrigin {
    async fn new(routes: axum::Router) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}/history.html", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
        Self { base_url, server }
    }
}

fn routes(requests: Arc<AtomicUsize>, request_received: Arc<tokio::sync::Notify>) -> axum::Router {
    axum::Router::new()
        .route(
            "/ancestor-source",
            axum::routing::get(|| async {
                axum::response::Html(format!(
                    r#"<!doctype html><body><script>
                    {PROBE}
                    addEventListener('message', event => {{
                        if (event.data.kind !== 'ancestor-probe') return;
                        const {{operation, target, destination}} = event.data;
                        event.source.postMessage({{
                            kind: 'ancestor-result',
                            value: ancestorNavigationProbe(operation, target, destination),
                        }}, '*');
                    }});
                </script>"#
                ))
            }),
        )
        .route(
            "/ancestor-parent",
            axum::routing::get(|| async { axum::response::Html(PARENT) }),
        )
        .route(
            "/ancestor-destination",
            axum::routing::any(move || {
                requests.fetch_add(1, Ordering::SeqCst);
                request_received.notify_one();
                async { axum::http::StatusCode::NO_CONTENT }
            }),
        )
}

#[derive(Debug, Default)]
struct Case {
    nested: bool,
    parent_cross_origin: bool,
    source_same_origin: bool,
    target_parent: bool,
    related_destination: bool,
    activation: &'static str,
    sandbox: Option<&'static str>,
    remove_sandbox: bool,
    popup: bool,
    named: bool,
}

async fn check(operation: &str, case: Case, allowed: bool) {
    tokio::task::LocalSet::new()
        .run_until(check_in_local_set(operation, case, allowed))
        .await;
}

async fn check_in_local_set(operation: &str, case: Case, allowed: bool) {
    let requests = Arc::new(AtomicUsize::new(0));
    let request_received = Arc::new(tokio::sync::Notify::new());
    let other = RemoteOrigin::new(routes(requests.clone(), request_received.clone())).await;
    let mut page =
        SameDocumentPage::with_routes(routes(requests.clone(), request_received.clone())).await;
    page.ctx.enable_background_navigation_scheduler_for_test();
    let source_base = if case.source_same_origin {
        &page.base_url
    } else {
        &other.base_url
    };
    let source_url = source_base.replace("/history.html", "/ancestor-source");
    let mut frame_url = url::Url::parse(&source_url).unwrap();
    if case.nested {
        let parent_base = if case.parent_cross_origin {
            &other.base_url
        } else {
            &page.base_url
        };
        frame_url =
            url::Url::parse(&parent_base.replace("/history.html", "/ancestor-parent")).unwrap();
        frame_url
            .query_pairs_mut()
            .append_pair("source", &source_url);
        if let Some(sandbox) = case.sandbox {
            frame_url.query_pairs_mut().append_pair("sandbox", sandbox);
        }
    }
    page.evaluate(&format!(
        r#"(async () => {{
            globalThis.ancestorContainer = {};
            ancestorContainer.beforeUnloadCount = 0;
            ancestorContainer.addEventListener('beforeunload', () => ++ancestorContainer.beforeUnloadCount);
            const frame = ancestorContainer.document.createElement('iframe');
            frame.id = 'source';
            frame.name = 'ancestor-parent';
            const sandbox = {};
            if (sandbox !== null) frame.setAttribute('sandbox', sandbox);
            frame.src = {};
            const loaded = new Promise(resolve => frame.onload = resolve);
            ancestorContainer.document.body.append(frame);
            await loaded;
            return true;
        }})()"#,
        if case.popup { "open('about:blank', 'ancestor-top')" } else { "window" },
        json!(if case.nested { None } else { case.sandbox }),
        json!(frame_url.as_str()),
    )).await;
    page.command("Runtime.enable", json!({})).await;
    let context =
        page.ctx
            .sent
            .iter()
            .rev()
            .find(|event| {
                event["method"] == "Runtime.executionContextCreated"
                    && event["params"]["context"]["name"] == source_url
                    && event["params"]["context"]["auxData"]["isDefault"] == true
            })
            .unwrap_or_else(|| panic!("missing source realm: {}", json!(page.ctx.sent)))["params"]
            ["context"]["id"]
            .clone();
    if case.remove_sandbox {
        page.evaluate(
            "ancestorContainer.document.getElementById('source').removeAttribute('sandbox')",
        )
        .await;
    }
    if case.activation == "parent" {
        page.command(
            "Runtime.evaluate",
            json!({"expression": "navigator.userActivation.hasBeenActive", "userGesture": true}),
        )
        .await;
    }
    if case.activation == "synthetic" {
        page.command(
            "Runtime.evaluate",
            json!({
                "expression": "document.body.appendChild(document.createElement('button')).click()",
                "contextId": context,
            }),
        )
        .await;
    }
    let destination_base = if case.related_destination {
        &page.base_url
    } else {
        &other.base_url
    };
    let destination = destination_base.replace("/history.html", "/ancestor-destination");
    let target = match (case.target_parent, case.named) {
        (true, false) => "_parent",
        (false, false) => "_top",
        (true, true) => "ancestor-parent",
        (false, true) => "ancestor-top",
    };
    if case.activation == "source" {
        page.command(
            "Runtime.evaluate",
            json!({
                "expression": "navigator.userActivation.hasBeenActive",
                "contextId": context,
                "userGesture": true,
            }),
        )
        .await;
    }
    let result = if case.popup {
        // Exercise the child realm directly for virtual popup windows, whose
        // message delivery is managed separately from the opener's page.
        let response = page
            .command(
                "Runtime.evaluate",
                json!({
                    "expression": format!(
                        "ancestorNavigationProbe({operation:?}, {target:?}, {destination:?})"
                    ),
                    "contextId": context,
                    "returnByValue": true,
                }),
            )
            .await;
        assert!(response["exceptionDetails"].is_null(), "{response}");
        response["result"]["value"].clone()
    } else {
        page.evaluate(&format!(
            r#"new Promise(resolve => {{
        const receive = event => {{
            if (event.data.kind !== 'ancestor-result') return;
            removeEventListener('message', receive);
            resolve(event.data.value);
        }};
        addEventListener('message', receive);
        let source = ancestorContainer.document.getElementById('source').contentWindow;
        if ({}) source = source[0];
        source.postMessage({{
            kind: 'ancestor-probe', operation: {operation:?},
            target: {target:?}, destination: {destination:?},
        }}, '*');
    }})"#,
            case.nested
        ))
        .await
    };
    let location = matches!(operation, "location" | "href" | "replace");
    assert_eq!(
        result["outcome"],
        if !allowed && location {
            "SecurityError"
        } else {
            "returned"
        },
        "{operation}/{case:?}: {result}"
    );
    assert_eq!(result["conversions"], usize::from(location));
    if !allowed && location {
        assert_eq!(result["localException"], true);
    }
    if operation == "open" {
        assert_eq!(result["returnedTarget"], allowed);
    }
    assert_eq!(result["before"][1], case.activation == "source", "{result}");
    if allowed {
        let description = format!("admitted ancestor navigation request: {operation}/{case:?}");
        // The HTTP server has its own wake source. A 204 response need not
        // publish another CDP event after the request counter changes.
        tokio::select! {
            () = page.ctx.wait_until_scheduler_state(
                &description,
                |_| requests.load(Ordering::SeqCst) == 1,
            ) => {},
            () = request_received.notified() => {},
        }
        assert_eq!(requests.load(Ordering::SeqCst), 1, "{operation}/{case:?}");
    } else {
        page.evaluate("new Promise(resolve => setTimeout(resolve, 50))")
            .await;
        assert_eq!(requests.load(Ordering::SeqCst), 0, "{operation}");
        assert_eq!(
            page.evaluate("ancestorContainer.beforeUnloadCount").await,
            0
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ancestor_navigation_rejects_cross_origin_sources_across_all_entry_points() {
    for operation in [
        "location", "href", "replace", "open", "anchor", "form", "post",
    ] {
        check(operation, Case::default(), false).await;
        check(
            operation,
            Case {
                nested: true,
                target_parent: true,
                ..Case::default()
            },
            false,
        )
        .await;
        check(
            operation,
            Case {
                popup: true,
                ..Case::default()
            },
            false,
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ancestor_navigation_rejects_named_ancestor_targets() {
    for operation in ["open", "anchor", "form", "post"] {
        check(
            operation,
            Case {
                popup: true,
                named: true,
                ..Case::default()
            },
            false,
        )
        .await;
        check(
            operation,
            Case {
                nested: true,
                target_parent: true,
                named: true,
                ..Case::default()
            },
            false,
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ancestor_navigation_preserves_accessible_ancestors_activation_and_related_destinations() {
    for operation in ["href", "open", "anchor", "form", "post"] {
        check(
            operation,
            Case {
                activation: "source",
                ..Case::default()
            },
            true,
        )
        .await;
        check(
            operation,
            Case {
                related_destination: true,
                ..Case::default()
            },
            true,
        )
        .await;
        check(
            operation,
            Case {
                nested: true,
                parent_cross_origin: true,
                source_same_origin: true,
                target_parent: true,
                ..Case::default()
            },
            true,
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ancestor_navigation_activation_belongs_to_the_source_and_only_unlocks_the_top() {
    for activation in ["parent", "synthetic"] {
        check(
            "href",
            Case {
                activation,
                ..Case::default()
            },
            false,
        )
        .await;
    }
    check(
        "href",
        Case {
            nested: true,
            target_parent: true,
            activation: "source",
            ..Case::default()
        },
        false,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ancestor_navigation_uses_committed_sandbox_flags_and_ancestor_delegation() {
    for (sandbox, needs_activation) in [
        ("allow-scripts allow-same-origin", None),
        (
            "allow-scripts allow-same-origin allow-top-navigation",
            Some(false),
        ),
        (
            "allow-scripts allow-same-origin allow-top-navigation-by-user-activation",
            Some(true),
        ),
    ] {
        for activation in ["none", "source"] {
            let allowed =
                needs_activation.is_some_and(|required| !required || activation == "source");
            check(
                "href",
                Case {
                    sandbox: Some(sandbox),
                    activation,
                    ..Case::default()
                },
                allowed,
            )
            .await;
        }
    }
    check(
        "href",
        Case {
            sandbox: Some("allow-scripts allow-same-origin"),
            remove_sandbox: true,
            ..Case::default()
        },
        false,
    )
    .await;
    check(
        "href",
        Case {
            nested: true,
            parent_cross_origin: true,
            sandbox: Some("allow-scripts allow-same-origin allow-top-navigation"),
            ..Case::default()
        },
        false,
    )
    .await;
}
