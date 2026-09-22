use super::super::tests_cdp_smoke_fixture::SmokeFixtureServer;
use super::super::*;
use super::support::CdpPageHarness;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::time::Duration;
use url::Url;

struct HistoryPage {
    ctx: TestContext,
    cdp: CdpPageHarness,
    next_id: u64,
}

impl HistoryPage {
    async fn evaluate(&mut self, expression: &str) -> Result<Value> {
        self.evaluate_with_await(expression, false).await
    }

    async fn evaluate_await(&mut self, expression: &str) -> Result<Value> {
        self.evaluate_with_await(expression, true).await
    }

    async fn evaluate_with_await(
        &mut self,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let response = if await_promise {
            self.cdp
                .evaluate_await_value(&mut self.ctx, id, expression)
                .await
        } else {
            self.cdp.evaluate_value(&mut self.ctx, id, expression).await
        };
        anyhow::ensure!(
            response.get("error").is_none() && response["result"].get("exceptionDetails").is_none(),
            "evaluating {expression}: {response}"
        );
        Ok(response["result"]["result"].clone())
    }
}

fn markup_url(server: &SmokeFixtureServer, markup: &str) -> String {
    let mut url = Url::parse(&server.url("/history-markup")).unwrap();
    url.query_pairs_mut().append_pair(
        "markup",
        &markup.replace(
            "<body>",
            "<head><script>onload=()=>document.body.dataset.loaded='yes';</script></head><body>",
        ),
    );
    url.into()
}

fn frame(url: &str, name: &str) -> String {
    format!(
        "<iframe data-static name=\"{name}\" src=\"{}\"></iframe>",
        url.replace('&', "&amp;").replace('"', "&quot;")
    )
}

async fn value(page: &mut HistoryPage, expression: &str) -> Result<Value> {
    let result = page
        .evaluate(&format!("JSON.stringify({expression})"))
        .await?;
    Ok(serde_json::from_str(
        result["value"]
            .as_str()
            .unwrap_or_else(|| panic!("{expression}: {result}")),
    )?)
}

async fn wait(page: &mut HistoryPage, expression: &str) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let result = page.evaluate(expression).await;
        if result.as_ref().is_ok_and(|value| value["value"] == true) {
            return Ok(());
        }
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {expression}: {result:?}"
        );
        page.ctx.complete_one_ready_scheduler_input_for_test().await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn navigate(page: &mut HistoryPage, root: &str, action: &str, url: &str) -> Result<()> {
    page.evaluate(&format!("({root}).{action}; true")).await?;
    wait(
        page,
        &format!(
            "({root}).location.href === {} && ({root}).document.body?.dataset.loaded === 'yes'",
            serde_json::to_string(url)?
        ),
    )
    .await
    .with_context(|| format!("after {root}.{action}"))
}

async fn open_root(
    url: &str,
    popup: bool,
    opener_url: &str,
) -> Result<(HistoryPage, &'static str)> {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let cdp = CdpPageHarness::attach(&mut ctx, 220_000).await;
    let response = cdp
        .navigate(&mut ctx, 220_005, if popup { opener_url } else { url })
        .await;
    anyhow::ensure!(response.get("error").is_none(), "{response}");
    let mut page = HistoryPage {
        ctx,
        cdp,
        next_id: 220_010,
    };
    let root = if popup { "testPopup" } else { "window" };
    if popup {
        page.evaluate(&format!(
            "globalThis.testPopup = open({}); true",
            serde_json::to_string(url)?
        ))
        .await?;
    }
    wait(
        &mut page,
        &format!(
            "({root}).location.href === {} && ({root}).document.body?.dataset.loaded === 'yes'",
            serde_json::to_string(url)?
        ),
    )
    .await
    .with_context(|| format!("opening {root}"))?;
    Ok((page, root))
}

fn frames(root: &str) -> String {
    format!("Array.from(({root}).document.querySelectorAll('iframe[data-static]'))")
}

async fn snapshot(page: &mut HistoryPage, root: &str) -> Result<Value> {
    value(page, &format!("({{length:({root}).history.length, frames:{}.map(f=>({{url:f.contentWindow.location.href, text:f.contentDocument.body.textContent, state:f.contentWindow.history.state, navigationState:f.contentWindow.navigation.currentEntry.getState() ?? null, id:f.contentWindow.navigation.currentEntry.id, keys:f.contentWindow.navigation.entries().map(e=>e.key)}}))}})", frames(root))).await
}

async fn srcdoc(page: &mut HistoryPage, frame: &str, markup: &str) -> Result<()> {
    page.evaluate_await(&format!(
        "(async()=>{{const f=({frame}); const loaded=new Promise(resolve=>f.addEventListener('load',()=>setTimeout(resolve,0),{{once:true}})); f.srcdoc={}; await loaded; return true;}})()",
        serde_json::to_string(markup)?)).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_history_restores_interleaved_frames_after_parent_document_replacement() {
    // Traversal drives a replacement load through the protocol output projector,
    // so use the same stack allowance as the other deep CDP tests.
    target_8mb_stack("nested-history-interleaved", || async {
        check_interleaved_frames_after_parent_document_replacement()
            .await
            .expect("interleaved frame history should survive parent document replacement");
    })
    .await;
}

async fn check_interleaved_frames_after_parent_document_replacement() -> Result<()> {
    let server = SmokeFixtureServer::start().await;
    let initial_url = markup_url(&server, "<!doctype html><body><p>initial</p>");
    for popup in [false, true] {
        for layout in ["named", "unnamed", "dynamic"] {
            let dynamic = if layout == "dynamic" {
                format!(
                    "<script>const f=document.createElement('iframe');f.name='a';f.src={};document.body.prepend(f);</script>",
                    serde_json::to_string(&initial_url)?
                )
            } else {
                String::new()
            };
            let names = if layout == "unnamed" {
                ["", ""]
            } else {
                ["a", "b"]
            };
            let source = markup_url(
                &server,
                &format!(
                    "<!doctype html><body>{dynamic}{}{}",
                    frame(&initial_url, names[0]),
                    frame(&initial_url, names[1])
                ),
            );
            let away = markup_url(&server, "<!doctype html><body><p>away from parent</p>");
            let (mut page, root) = open_root(&source, popup, &server.url("/plain?opener")).await?;
            let list = frames(root);
            let initial = snapshot(&mut page, root)
                .await
                .with_context(|| format!("popup={popup}, {layout}: initial frames"))?;
            let mut stages = vec![initial.clone()];
            for (index, text) in [
                (0, "first historical source"),
                (1, "second historical source"),
            ] {
                srcdoc(
                    &mut page,
                    &format!("({list})[{index}]"),
                    &format!("<p>{text}</p>"),
                )
                .await?;
                wait(
                    &mut page,
                    &format!("({list})[{index}].contentDocument.body.textContent === '{text}'"),
                )
                .await?;
                stages.push(snapshot(&mut page, root).await.with_context(|| {
                    format!("popup={popup}, {layout}: frame {index} srcdoc loaded")
                })?);
            }
            page.evaluate(&format!(
                "({list})[0].contentWindow.location.hash = 'fragment'; true"
            ))
            .await?;
            wait(
                &mut page,
                &format!("({list})[0].contentWindow.location.hash === '#fragment'"),
            )
            .await?;
            page.evaluate(&format!("({list})[0].contentWindow.history.replaceState({{classic:3}}, ''); ({list})[0].contentWindow.navigation.updateCurrentEntry({{state:{{navigation:3}}}}); true")).await?;
            let expected = snapshot(&mut page, root)
                .await
                .with_context(|| format!("popup={popup}, {layout}: fragment state updated"))?;
            assert_eq!(
                expected["length"].as_u64(),
                initial["length"].as_u64().map(|length| length + 3)
            );

            navigate(
                &mut page,
                root,
                &format!("location.href = {}", serde_json::to_string(&away)?),
                &away,
            )
            .await?;
            navigate(&mut page, root, "history.back()", &source).await?;
            let restored = snapshot(&mut page, root)
                .await
                .with_context(|| format!("popup={popup}, {layout}: parent restored"))?;
            assert_eq!(
                restored["frames"], expected["frames"],
                "popup={popup}, {layout}: {restored}"
            );
            assert_eq!(
                restored["length"].as_u64(),
                initial["length"].as_u64().map(|length| length + 4)
            );
            for reference in stages.iter().rev() {
                page.evaluate(&format!("({root}).history.back(); true"))
                    .await?;
                let urls = reference["frames"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|frame| frame["url"].clone())
                    .collect::<Vec<_>>();
                // The URL changes at commit, before the restored child document
                // finishes parsing. Read its body only after that load completes.
                wait(
                    &mut page,
                    &format!(
                        "({list}).every(f=>f.contentDocument?.readyState === 'complete') && JSON.stringify(({list}).map(f=>f.contentWindow.location.href)) === {}",
                        serde_json::to_string(&serde_json::to_string(&urls)?)?
                    ),
                )
                .await?;
                let current = snapshot(&mut page, root).await.with_context(|| {
                    format!("popup={popup}, {layout}: traversed child entries {urls:?}")
                })?;
                for index in 0..2 {
                    assert_eq!(
                        current["frames"][index]["id"], reference["frames"][index]["id"],
                        "popup={popup}, {layout}: {current}"
                    );
                }
            }
            navigate(&mut page, root, "history.go(4)", &away).await?;
            navigate(&mut page, root, "history.go(-3)", &source).await?;
            let restored = snapshot(&mut page, root)
                .await
                .with_context(|| format!("popup={popup}, {layout}: parent restored by delta"))?;
            for index in 0..2 {
                assert_eq!(
                    restored["frames"][index]["id"], stages[1]["frames"][index]["id"],
                    "popup={popup}, {layout}: {restored}"
                );
            }
            page.evaluate(&format!(
                "({list})[1].contentWindow.history.pushState({{fork:true}}, '', '#fork'); true"
            ))
            .await?;
            assert_eq!(
                value(&mut page, &format!("({root}).history.length")).await?,
                json!(initial["length"].as_u64().unwrap() + 2)
            );
            if layout == "dynamic" {
                assert_eq!(value(&mut page, &format!("({root}).document.querySelector('iframe').contentDocument.body.textContent")).await?, json!("initial"));
            }
            if popup {
                page.evaluate("testPopup.close(); true").await?;
            }
        }
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_history_retains_children_of_a_script_created_parent_navigable() -> Result<()> {
    let server = SmokeFixtureServer::start().await;
    let leaf = markup_url(&server, "<!doctype html><body><p>initial</p>");
    let parent = markup_url(
        &server,
        &format!("<!doctype html><body>{}", frame(&leaf, "grandchild")),
    );
    let source = markup_url(
        &server,
        &format!(
            "<!doctype html><body><script>const f=document.createElement('iframe');f.src={};document.body.append(f);</script>",
            serde_json::to_string(&parent)?
        ),
    );
    let away = markup_url(&server, "<!doctype html><body><p>away from child</p>");
    for popup in [false, true] {
        let (mut page, root) = open_root(&source, popup, &server.url("/plain?opener")).await?;
        let container = format!("({root}).document.querySelector('iframe').contentWindow");
        let child = format!("({container}).document.querySelector('iframe')");
        srcdoc(&mut page, &child, "<p>script parent historical child</p>").await?;
        let id = value(
            &mut page,
            &format!("({child}).contentWindow.navigation.currentEntry.id"),
        )
        .await?;
        navigate(
            &mut page,
            &container,
            &format!("location.href = {}", serde_json::to_string(&away)?),
            &away,
        )
        .await?;
        navigate(&mut page, &container, "history.back()", &parent).await?;
        assert_eq!(
            value(
                &mut page,
                &format!("({child}).contentWindow.navigation.currentEntry.id")
            )
            .await?,
            id
        );
        assert_eq!(
            value(
                &mut page,
                &format!("({child}).contentDocument.body.textContent")
            )
            .await?,
            json!("script parent historical child")
        );
        if popup {
            page.evaluate("testPopup.close(); true").await?;
        }
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_history_restores_grandchildren_with_their_parent_document_identity() {
    target_8mb_stack("nested-history-grandchildren", || async {
        check_grandchildren_with_their_parent_document_identity()
            .await
            .expect("grandchild history should keep its parent document identity");
    })
    .await;
}

async fn check_grandchildren_with_their_parent_document_identity() -> Result<()> {
    let server = SmokeFixtureServer::start().await;
    let leaf = markup_url(&server, "<!doctype html><body><p>initial</p>");
    let parent = markup_url(
        &server,
        &format!("<!doctype html><body>{}", frame(&leaf, "grandchild")),
    );
    let source = markup_url(
        &server,
        &format!("<!doctype html><body>{}", frame(&parent, "child")),
    );
    let away = markup_url(&server, "<!doctype html><body><p>away from grandchild</p>");
    for popup in [false, true] {
        let (mut page, root) = open_root(&source, popup, &server.url("/plain?opener")).await?;
        let child = format!(
            "({root}).document.querySelector('iframe').contentDocument.querySelector('iframe')"
        );
        srcdoc(&mut page, &child, "<p>historical grandchild</p>").await?;
        wait(
            &mut page,
            &format!("({child}).contentDocument.body.textContent === 'historical grandchild'"),
        )
        .await?;
        let id = value(
            &mut page,
            &format!("({child}).contentWindow.navigation.currentEntry.id"),
        )
        .await?;
        navigate(
            &mut page,
            root,
            &format!("location.href = {}", serde_json::to_string(&away)?),
            &away,
        )
        .await?;
        navigate(&mut page, root, "history.back()", &source).await?;
        assert_eq!(
            value(
                &mut page,
                &format!("({child}).contentWindow.navigation.currentEntry.id")
            )
            .await?,
            id
        );
        assert_eq!(
            value(
                &mut page,
                &format!("({child}).contentDocument.body.textContent")
            )
            .await?,
            json!("historical grandchild")
        );
        if popup {
            page.evaluate("testPopup.close(); true").await?;
        }
    }

    Ok(())
}
