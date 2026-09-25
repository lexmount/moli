use super::*;

const SETUP: &str = include_str!("hyperlink_navigation.js");

async fn activate(
    page: &mut SameDocumentPage,
    target: &str,
    mode: &str,
    destination: &str,
    action: &str,
) {
    page.evaluate(&format!(
        "{SETUP}\nsetupClickProbe({target:?}, {mode:?}, {destination:?}, {action:?})"
    ))
    .await;
    if matches!(mode, "trusted" | "location" | "nested-click") {
        page.command("Page.captureScreenshot", json!({})).await;
        for (kind, buttons) in [("mousePressed", 1), ("mouseReleased", 0)] {
            page.command("Input.dispatchMouseEvent", json!({
                "type": kind, "x": 20, "y": 20, "button": "left", "buttons": buttons, "clickCount": 1,
            })).await;
        }
    }
    page.evaluate("new Promise(resolve => setTimeout(resolve, 0))")
        .await;
}

async fn check_click_involvement(target: &str) {
    for destination in ["fragment", "document"] {
        for mode in ["trusted", "script", "dispatch", "location", "nested-click"] {
            let mut page = SameDocumentPage::new().await;
            activate(&mut page, target, mode, destination, "cancel").await;
            let result = page.evaluate(r#"({records, clicks, unchanged: probeTarget.location.href === probeInitialURL && probeTarget.document === probeInitialDocument})"#).await;
            let clicks = match mode {
                "trusted" | "location" => json!([true]),
                "nested-click" => json!([true, false]),
                _ => json!([false]),
            };
            assert_eq!(
                result,
                json!({
                    "records": [{
                        "userInitiated": mode == "trusted", "source": mode != "location", "sourceNull": mode == "location",
                        "sameDocument": destination == "fragment", "hashChange": destination == "fragment", "type": "push",
                    }], "clicks": clicks, "unchanged": true,
                }),
                "{target}/{destination}/{mode}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hyperlink_navigation_top_click_involvement() {
    check_click_involvement("top").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hyperlink_navigation_child_click_involvement() {
    check_click_involvement("child").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hyperlink_navigation_named_child_click_involvement() {
    check_click_involvement("named-child").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hyperlink_navigation_named_popup_click_involvement() {
    check_click_involvement("named-popup").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hyperlink_navigation_commits_and_intercepts_existing_targets() {
    for target in ["top", "child", "named-child", "named-popup"] {
        for (destination, action) in [("fragment", "proceed"), ("document", "intercept")] {
            let mut page = SameDocumentPage::new().await;
            activate(&mut page, target, "trusted", destination, action).await;
            let result = page.evaluate(r#"({
                reached: probeTarget.location.href === probeDestination,
                retained: probeTarget.document === probeInitialDocument,
                success: probeSuccess, events: records.length, userInitiated: records[0]?.userInitiated,
            })"#).await;
            assert_eq!(
                result,
                json!({"reached": true, "retained": true, "success": true, "events": 1, "userInitiated": true}),
                "{target}/{destination}/{action}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hyperlink_navigation_child_ancestor_click_involvement() {
    for target in ["child-top", "child-parent"] {
        check_click_involvement(target).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hyperlink_navigation_named_popup_preserves_referrer_policy() {
    for policy in ["no-referrer", "origin"] {
        let route = axum::Router::new().route("/referrer.html", axum::routing::get(|headers: axum::http::HeaderMap| async move {
            let referer = headers.get("referer").and_then(|value| value.to_str().ok()).unwrap_or_default();
            axum::response::Html(format!(r#"<!doctype html><script>opener.postMessage({{type:'referrer-report', referrer:document.referrer, header:{}}}, '*')</script>"#, serde_json::to_string(referer).unwrap()))
        }));
        let mut page = SameDocumentPage::with_routes(route).await;
        let result = page.evaluate(&format!(r#"(async () => {{
            const popup = open('?popup', 'referrer-target');
            await new Promise(resolve => popup.addEventListener('load', resolve, {{once:true}}));
            const link = document.createElement('a');
            link.href = '/referrer.html'; link.target = 'referrer-target'; link.referrerPolicy = {policy:?};
            document.body.append(link);
            const events = [];
            popup.navigation.onnavigate = event => events.push(event.sourceElement === link);
            const report = new Promise(resolve => addEventListener('message', event => {{
                if (event.data?.type === 'referrer-report') resolve(event.data);
            }}));
            link.click();
            return {{report:await report, events}};
        }})()"#)).await;
        let expected = if policy == "no-referrer" {
            String::new()
        } else {
            format!("{}/", page.base_url.trim_end_matches("/history.html"))
        };
        assert_eq!(result["report"]["referrer"], expected, "{policy}: {result}");
        assert_eq!(result["events"], json!([true]), "{policy}: {result}");
        if policy == "no-referrer" {
            assert_eq!(result["report"]["header"], "", "{result}");
        }
    }
}
