use super::*;

const SESSION: &str = "SID-COMMIT-HISTORY";
const FRAME: &str = "TID-COMMIT-HISTORY";

struct SameDocumentPage {
    ctx: TestContext,
    base_url: String,
    browser_base: usize,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for SameDocumentPage {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl SameDocumentPage {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let app = axum::Router::new().route(
                "/history.html",
                axum::routing::get(|| async {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><title>History commits</title>",
                    )
                }),
            );
            axum::serve(listener, app).await.unwrap();
        });
        let mut ctx = TestContext::new();
        load_bc_with_session(
            &mut ctx,
            "BID-COMMIT-HISTORY",
            FRAME,
            SESSION,
            "about:blank",
        );
        ctx.enable_page_events_for_test(Some(SESSION));
        let mut page = Self {
            ctx,
            base_url: format!("http://{addr}/history.html"),
            browser_base: 0,
            server,
        };
        page.command("Page.navigate", json!({ "url": page.base_url }))
            .await;
        wait_until_frame_stopped_loading(&mut page.ctx, FRAME).await;
        let history = page.command("Page.getNavigationHistory", json!({})).await;
        page.browser_base = history["currentIndex"].as_u64().unwrap() as usize;
        page.evaluate(
            r#"
                globalThis.observations = [];
                navigation.addEventListener('currententrychange', () => {
                    observations.push([location.href, document.URL, navigation.currentEntry.url]);
                });
                for (const type of ['popstate', 'hashchange']) {
                    addEventListener(type, () => {
                        observations.push([location.href, document.URL, navigation.currentEntry.url]);
                    });
                }
                void 0;
            "#,
        ).await;
        page.ctx.sent.clear();
        page
    }

    async fn command(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        self.ctx
            .process_and_wait_for_response_async(json!({
                "id": 1001,
                "method": method,
                "sessionId": SESSION,
                "params": params,
            }))
            .await;
        let response = take_response_by_id(&mut self.ctx, 1001);
        assert!(response["error"].is_null(), "{method}: {response}");
        response["result"].clone()
    }

    async fn evaluate(&mut self, expression: &str) -> serde_json::Value {
        let result = self
            .command(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                }),
            )
            .await;
        assert!(
            result["exceptionDetails"].is_null(),
            "{expression}: {result}"
        );
        result["result"]["value"].clone()
    }

    async fn run(&mut self, expression: &str, final_fragment: &str) {
        self.evaluate(expression).await;
        let expected_url = format!("{}{final_fragment}", self.base_url);
        wait_until_message(&mut self.ctx, SESSION, "same-document commit", |message| {
            message["method"] == "Page.navigatedWithinDocument"
                && message["params"]["url"] == expected_url
        })
        .await;
        self.evaluate("new Promise(resolve => setTimeout(resolve, 0))")
            .await;
    }

    async fn assert_history(&mut self, fragments: &[&str], index: usize) {
        let urls = fragments
            .iter()
            .map(|fragment| format!("{}{fragment}", self.base_url))
            .collect::<Vec<_>>();
        let renderer = self
            .evaluate(
                r#"({
            urls: navigation.entries().map(entry => entry.url),
            index: navigation.currentEntry.index,
            href: location.href,
            documentURL: document.URL,
            observations,
        })"#,
            )
            .await;
        assert_eq!(renderer["urls"], json!(urls));
        assert_eq!(renderer["index"], json!(index));
        assert_eq!(renderer["href"], json!(urls[index]));
        assert_eq!(renderer["documentURL"], renderer["href"]);
        for observation in renderer["observations"].as_array().unwrap() {
            assert_eq!(
                observation[0], observation[1],
                "Document.URL during callback"
            );
            assert_eq!(
                observation[0], observation[2],
                "current entry during callback"
            );
        }
        let browser = self.command("Page.getNavigationHistory", json!({})).await;
        assert_eq!(browser["currentIndex"], json!(self.browser_base + index));
        let browser_urls = browser["entries"].as_array().unwrap()[self.browser_base..]
            .iter()
            .map(|entry| entry["url"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            browser_urls, urls,
            "browser history must preserve renderer commit order"
        );
    }

    fn assert_commits(&mut self, expected: &[(&str, &str)]) {
        let actual = self
            .ctx
            .sent
            .iter()
            .filter(|message| message["method"] == "Page.navigatedWithinDocument")
            .map(|message| {
                assert_eq!(message["params"]["frameId"], FRAME);
                (
                    message["params"]["url"].as_str().unwrap().to_owned(),
                    message["params"]["navigationType"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                )
            })
            .collect::<Vec<_>>();
        let expected = expected
            .iter()
            .map(|(fragment, kind)| (format!("{}{fragment}", self.base_url), (*kind).to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        self.ctx.sent.clear();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn same_document_commits_keep_navigation_api_and_browser_history_in_sync() {
    let mut page = SameDocumentPage::new().await;
    page.run("navigation.navigate('#one').finished", "#one")
        .await;
    page.assert_history(&["", "#one"], 1).await;
    page.assert_commits(&[("#one", "fragment")]);

    page.run(
        "navigation.navigate('#two', {history: 'replace'}).finished",
        "#two",
    )
    .await;
    page.assert_history(&["", "#two"], 1).await;
    page.assert_commits(&[("#two", "fragment")]);

    page.run("location.hash = 'three'; void 0", "#three").await;
    page.assert_history(&["", "#two", "#three"], 2).await;
    page.assert_commits(&[("#three", "fragment")]);

    page.run("navigation.back().finished", "#two").await;
    page.assert_history(&["", "#two", "#three"], 1).await;
    page.assert_commits(&[("#two", "fragment")]);

    page.run("history.forward()", "#three").await;
    page.assert_history(&["", "#two", "#three"], 2).await;
    page.assert_commits(&[("#three", "fragment")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn same_document_commits_precede_reentrant_author_callbacks() {
    for (outer, first_kind, listener, nested, second_kind) in [
        (
            "navigation.navigate('#one').finished.catch(() => {})",
            "fragment",
            "navigation.addEventListener('currententrychange',",
            "history.pushState(null, '', '#two')",
            "historyApi",
        ),
        (
            "navigation.navigate('#one').finished.catch(() => {})",
            "fragment",
            "navigation.addEventListener('currententrychange',",
            "location.hash = 'two'",
            "fragment",
        ),
        (
            "history.pushState(null, '', '#one')",
            "historyApi",
            "navigation.addEventListener('currententrychange',",
            "navigation.navigate('#two').finished.catch(() => {})",
            "fragment",
        ),
        (
            "history.pushState(null, '', '#one')",
            "historyApi",
            "navigation.addEventListener('currententrychange',",
            "location.hash = 'two'",
            "fragment",
        ),
        (
            "location.hash = 'one'",
            "fragment",
            "navigation.addEventListener('currententrychange',",
            "history.pushState(null, '', '#two')",
            "historyApi",
        ),
        (
            "location.hash = 'one'",
            "fragment",
            "addEventListener('popstate',",
            "history.pushState(null, '', '#two')",
            "historyApi",
        ),
        (
            "navigation.navigate('#one').finished.catch(() => {})",
            "fragment",
            "addEventListener('popstate',",
            "history.pushState(null, '', '#two')",
            "historyApi",
        ),
    ] {
        let mut page = SameDocumentPage::new().await;
        page.run(
            &format!("{listener} () => {{ {nested}; }}, {{once: true}}); {outer}; void 0"),
            "#two",
        )
        .await;
        page.assert_history(&["", "#one", "#two"], 2).await;
        page.assert_commits(&[("#one", first_kind), ("#two", second_kind)]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn same_document_commits_publish_traversal_before_nested_push_state() {
    for listener in [
        "navigation.addEventListener('currententrychange',",
        "addEventListener('popstate',",
    ] {
        let mut page = SameDocumentPage::new().await;
        page.run(
            "history.pushState(null, '', '#one'); history.pushState(null, '', '#two');",
            "#two",
        )
        .await;
        page.assert_commits(&[("#one", "historyApi"), ("#two", "historyApi")]);
        page.run(&format!("{listener} () => history.pushState(null, '', '#nested'), {{once: true}}); history.back();"), "#nested").await;
        page.assert_history(&["", "#one", "#nested"], 2).await;
        page.assert_commits(&[("#one", "fragment"), ("#nested", "historyApi")]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn same_document_commits_distinguish_interception_and_cancellation() {
    for navigate in [
        "navigation.navigate('#one').finished",
        "location.hash = 'one'; void 0",
    ] {
        let mut page = SameDocumentPage::new().await;
        page.run(&format!("navigation.addEventListener('navigate', e => e.intercept({{handler: () => Promise.resolve()}}), {{once: true}}); {navigate}"), "#one").await;
        page.assert_history(&["", "#one"], 1).await;
        page.assert_commits(&[("#one", "other")]);
    }

    for handler in [
        "handler: () => Promise.resolve()",
        "precommitHandler: () => Promise.resolve()",
    ] {
        let mut page = SameDocumentPage::new().await;
        page.run(
            "history.pushState(null, '', '#one'); history.pushState(null, '', '#two');",
            "#two",
        )
        .await;
        page.assert_commits(&[("#one", "historyApi"), ("#two", "historyApi")]);
        page.run(&format!("navigation.addEventListener('navigate', e => e.intercept({{{handler}}}), {{once: true}}); navigation.back().finished"), "#one").await;
        page.assert_history(&["", "#one", "#two"], 1).await;
        page.assert_commits(&[("#one", "other")]);
    }

    let mut page = SameDocumentPage::new().await;
    page.evaluate("navigation.addEventListener('navigate', e => e.preventDefault(), {once: true}); navigation.navigate('#one').finished.catch(e => e.name)").await;
    page.assert_history(&[""], 0).await;
    page.assert_commits(&[]);
}
