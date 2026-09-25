use super::*;

const PROBE: &str = include_str!("navigation_focus.js");
const CHILD: &str = include_str!("navigation_focus.html");

async fn focus_page() -> SameDocumentPage {
    SameDocumentPage::with_routes(
        axum::Router::new()
            .route(
                "/focus-child.html",
                axum::routing::get(|| async { axum::response::Html(CHILD) }),
            )
            .route(
                "/focus-denied.html",
                axum::routing::get(|| async {
                    (
                        [("permissions-policy", "focus-without-user-activation=()")],
                        axum::response::Html(CHILD),
                    )
                }),
            )
            .route(
                "/focus-parent-denied.html",
                axum::routing::get(|| async {
                    (
                        [("permissions-policy", "focus-without-user-activation=()")],
                        axum::response::Html("<!doctype html><body>"),
                    )
                }),
            )
            .route(
                "/focus-initial.html",
                axum::routing::get(|| async {
                    axum::response::Html(
                        "<!doctype html><button id='initial' autofocus>Initial</button>",
                    )
                }),
            ),
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_focus_reset_rescans_autofocus_after_initial_processing() {
    let mut page = focus_page().await;
    let url = page
        .base_url
        .replace("/history.html", "/focus-initial.html");
    page.command("Page.navigate", json!({"url": url})).await;
    wait_until_frame_stopped_loading(&mut page.ctx, FRAME).await;
    let result = page.evaluate(r##"(async () => {
        await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        const initial = document.activeElement.id;
        document.getElementById('initial').remove();
        document.body.insertAdjacentHTML('beforeend', '<button id="first" autofocus>First</button><button id="second" autofocus>Second</button><button id="decoy">Decoy</button>');
        const events = [];
        for (const button of document.querySelectorAll('button')) {
            button.addEventListener('focus', () => events.push(button.id));
        }
        document.getElementById('decoy').focus();
        navigation.onnavigate = event => event.intercept({});
        await navigation.navigate('#first').finished;
        const first = document.activeElement.id;
        document.getElementById('first').disabled = true;
        document.getElementById('decoy').focus();
        await navigation.navigate('#second').finished;
        return { initial, first, second: document.activeElement.id, events };
    })()"##).await;
    assert_eq!(
        result,
        json!({
            "initial": "initial", "first": "first", "second": "second",
            "events": ["decoy", "first", "decoy", "second"],
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_focus_reset_controls_tab_start_without_focusing_body() {
    for mode in ["reset", "manual", "none"] {
        for reverse in [false, true] {
            let mut page = focus_page().await;
            let initial = if reverse { "second" } else { "first" };
            let result = page.evaluate(&format!(r#"(async () => {{
                document.body.id = 'body';
                document.body.innerHTML = '<button id="first">First</button><button id="second">Second</button>';
                globalThis.bodyFocusEvents = 0;
                document.body.addEventListener('focus', () => ++bodyFocusEvents);
                document.getElementById({initial:?}).focus();
                document.body.focus();
                const afterBodyFocus = document.activeElement.id;
                const mode = {mode:?};
                if (mode !== 'none') navigation.onnavigate = event => event.intercept({{
                    focusReset: mode === 'manual' ? 'manual' : 'after-transition'
                }});
                await navigation.navigate('#next').finished;
                const afterNavigation = document.activeElement.id;
                document.body.focus();
                if (document.activeElement !== document.body) document.activeElement.blur();
                return {{ afterBodyFocus, afterNavigation, bodyFocusEvents }};
            }})()"#)).await;
            assert_eq!(
                result,
                json!({
                    "afterBodyFocus": initial,
                    "afterNavigation": if mode == "reset" { "body" } else { initial },
                    "bodyFocusEvents": 0,
                }),
                "{mode}/reverse={reverse}"
            );
            for kind in ["keyDown", "keyUp"] {
                page.command(
                    "Input.dispatchKeyEvent",
                    json!({
                        "type": kind, "key": "Tab", "code": "Tab", "windowsVirtualKeyCode": 9,
                        "modifiers": if reverse { 8 } else { 0 },
                    }),
                )
                .await;
            }
            let expected = match (mode == "reset", reverse) {
                (true, false) | (false, true) => "first",
                _ => "second",
            };
            assert_eq!(
                page.evaluate("document.activeElement.id").await,
                expected,
                "{mode}/reverse={reverse}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_focus_reset_respects_document_policy_and_manual_focus() {
    for (cross, allow, mode, child_denied, parent_denied, expected) in [
        (false, None, "script", false, false, "autofocus-input"),
        (true, None, "script", false, false, "child-body"),
        (false, Some("'none'"), "script", false, false, "child-body"),
        (true, Some("'none'"), "script", false, false, "child-body"),
        (true, Some("*"), "script", false, false, "autofocus-input"),
        (false, Some("*"), "manual", false, false, "child-body"),
        (true, Some("*"), "manual", false, false, "child-body"),
        (false, Some("*"), "change-focus", false, false, "decoy"),
        (true, Some("*"), "change-focus", false, false, "decoy"),
        (true, Some("*"), "script", true, false, "child-body"),
        (true, Some("*"), "script", false, true, "child-body"),
    ] {
        let mut page = focus_page().await;
        if parent_denied {
            let url = page
                .base_url
                .replace("/history.html", "/focus-parent-denied.html");
            page.command("Page.navigate", json!({"url": url})).await;
            wait_until_frame_stopped_loading(&mut page.ctx, FRAME).await;
        }
        let allow = serde_json::to_string(&allow).unwrap();
        let result = if parent_denied {
            page.evaluate(&format!(
                "{PROBE}\nsetupNavigationFocusProbe({cross}, {allow}, {child_denied})"
            ))
            .await;
            // Establish the parent's focus with real input even though its
            // policy also denies script-initiated focus without activation.
            page.command("Page.captureScreenshot", json!({})).await;
            for (kind, buttons) in [("mousePressed", 1), ("mouseReleased", 0)] {
                page.command("Input.dispatchMouseEvent", json!({
                    "type": kind, "x": 310, "y": 10, "button": "left", "buttons": buttons, "clickCount": 1,
                })).await;
            }
            page.evaluate("focusProbeFrame.contentWindow.postMessage({action:'navigate',mode:'script'}, '*'); focusProbeResult()")
                .await
        } else {
            page.evaluate(&format!(
                "{PROBE}\nrunNavigationFocusProbe({cross}, {allow}, {mode:?}, {child_denied})"
            ))
            .await
        };
        let case = format!(
            "{cross}/{allow}/{mode}/child-denied={child_denied}/parent-denied={parent_denied}"
        );
        let parent = if expected == "child-body" {
            "parent-input"
        } else {
            "focus-frame"
        };
        assert_eq!(result["parent"], parent, "{case}: {result}");
        // An inactive child can retain its previously focused element. The
        // denied/manual cases assert that navigation never takes parent focus.
        if expected != "child-body" {
            assert_eq!(result["active"], expected, "{case}: {result}");
        }
        assert_eq!(result["activation"], false, "{case}: {result}");
        assert_eq!(result["userInitiated"], false, "{case}: {result}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_focus_reset_preserves_trusted_initiation_after_activation_expires() {
    let mut page = focus_page().await;
    page.evaluate(&format!(
        "{PROBE}\nsetupNavigationFocusProbe(true, \"'none'\")"
    ))
    .await;
    page.command("Page.captureScreenshot", json!({})).await;
    for (kind, buttons) in [("mousePressed", 1), ("mouseReleased", 0)] {
        page.command("Input.dispatchMouseEvent", json!({
            "type": kind, "x": 20, "y": 20, "button": "left", "buttons": buttons, "clickCount": 1,
        })).await;
    }
    let result = page
        .evaluate(
            r#"(async () => {
        await focusProbeWait('started');
        document.getElementById('parent-input').focus();
        return focusProbeResult();
    })()"#,
        )
        .await;
    assert_eq!(
        result,
        json!({
            "active": "autofocus-input", "parent": "focus-frame",
            "activation": false, "userInitiated": true,
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_focus_reset_tracks_viewport_focus_in_its_own_document() {
    let mut page = focus_page().await;
    let result = page
        .evaluate(&format!("{PROBE}\nrunNavigationViewportFocusProbe()"))
        .await;
    assert_eq!(
        result,
        json!([
            { "mode": "child-viewport", "active": "button", "frameFocused": true },
            { "mode": "parent-viewport", "active": "autofocus", "frameFocused": true },
        ])
    );
}
