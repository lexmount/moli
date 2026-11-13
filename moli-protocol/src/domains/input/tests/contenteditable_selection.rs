use super::*;

async fn select_editor_in_isolated_world(ctx: &mut TestContext) {
    ctx.process_async(json!({"id": 10, "method": "Page.getFrameTree"}))
        .await;
    let frame_id = ctx.take_response_by_id(10)["result"]["frameTree"]["frame"]["id"]
        .as_str()
        .expect("main frame id")
        .to_owned();
    ctx.process_async(json!({
        "id": 11, "method": "Page.createIsolatedWorld",
        "params": {"frameId": frame_id, "worldName": "fill-utility-world"}
    }))
    .await;
    let context_id = ctx.take_response_by_id(11)["result"]["executionContextId"]
        .as_i64()
        .expect("isolated execution context id");
    ctx.process_async(json!({
        "id": 12, "method": "Runtime.evaluate",
        "params": {
            "contextId": context_id,
            "returnByValue": true,
            "expression": r#"(() => {
                const editor = document.getElementById('editor');
                editor.focus();
                const range = document.createRange();
                range.selectNodeContents(editor);
                const selection = getSelection();
                selection.removeAllRanges();
                selection.addRange(range);
                return [String(selection), selection.getRangeAt(0) === range];
            })()"#
        }
    }))
    .await;
    let response = ctx.take_response_by_id(12);
    assert!(
        response["result"]["exceptionDetails"].is_null(),
        "{response}"
    );
    assert_eq!(response["result"]["result"]["value"][1], true, "{response}");
}

#[tokio::test(flavor = "multi_thread")]
async fn contenteditable_isolated_selection_is_replaced_by_cdp_input() {
    for (content, original, value, setup) in [
        ("edit me", "edit me", "typed", ""),
        (
            "before <b>bold</b><i> after</i>",
            "before bold after",
            "typed",
            "",
        ),
        ("原文😀", "原文😀", "替换🌍", ""),
        ("edit me", "edit me", "", ""),
        (
            "edit me",
            "edit me",
            "typed",
            "editor.focus(); getSelection().setBaseAndExtent(editor.firstChild, 1, editor.firstChild, 3);",
        ),
        (
            "edit me",
            "edit me",
            "typed",
            "editor.addEventListener('focus', () => { getSelection().selectAllChildren(editor); getSelection().collapseToEnd(); });",
        ),
    ] {
        let mut ctx = TestContext::new();
        let html = r#"<html><head><meta charset=utf-8></head><body><div id=editor contenteditable=true>CONTENT</div>
            <script>
                const editor = document.getElementById('editor');
                window.inputEvents = [];
                for (const type of ['beforeinput', 'input']) editor.addEventListener(type, event => {
                    inputEvents.push([event.type, editor.textContent, String(getSelection())]);
                });
                SETUP
            </script></body></html>"#.replace("CONTENT", content).replace("SETUP", setup);
        with_loaded_document(&mut ctx, &html).await;
        select_editor_in_isolated_world(&mut ctx).await;
        assert_eq!(
            evaluate_string(&mut ctx, "String(getSelection())").await,
            original
        );

        if value.is_empty() {
            // Playwright fill('') deletes the utility world's selected contents.
            ctx.process_async(json!({
                "id": 13, "method": "Input.dispatchKeyEvent",
                "params": {"type": "keyDown", "key": "Delete", "code": "Delete", "windowsVirtualKeyCode": 46}
            })).await;
        } else {
            ctx.process_async(json!({
                "id": 13, "method": "Input.insertText", "params": {"text": value}
            }))
            .await;
        }
        ctx.expect_result(13, json!({}), None);
        assert_eq!(
            evaluate_string(&mut ctx, "document.getElementById('editor').textContent").await,
            value,
            "{content}: {setup}"
        );
        let events = evaluate_string(&mut ctx, "JSON.stringify(inputEvents)").await;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&events).unwrap(),
            json!([["beforeinput", original, original], ["input", value, ""]]),
            "{content}: {setup}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn contenteditable_isolated_selection_preserves_beforeinput_validation() {
    for (mutation, should_edit) in [
        ("event.preventDefault();", false),
        ("editor.remove();", false),
        ("editor.contentEditable='false';", false),
        (
            "document.implementation.createHTMLDocument('other').adoptNode(editor);",
            false,
        ),
        ("throw new Error('beforeinput-control');", true),
    ] {
        let mut ctx = TestContext::new();
        let html = r#"<html><head><meta charset=utf-8></head><body><div id=editor contenteditable=true>edit me</div>
            <script>
                window.editor = document.getElementById('editor');
                window.inputCount = 0;
                editor.addEventListener('beforeinput', event => { MUTATION });
                editor.addEventListener('input', () => inputCount++);
            </script></body></html>"#
            .replace("MUTATION", mutation);
        with_loaded_document(&mut ctx, &html).await;
        select_editor_in_isolated_world(&mut ctx).await;
        ctx.process_async(json!({
            "id": 13, "method": "Input.insertText", "params": {"text": "typed"}
        }))
        .await;
        ctx.expect_result(13, json!({}), None);
        let actual =
            evaluate_string(&mut ctx, "JSON.stringify([editor.textContent, inputCount])").await;
        let expected = if should_edit {
            json!(["typed", 1])
        } else {
            json!(["edit me", 0])
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
            expected,
            "{mutation}"
        );
    }
}
