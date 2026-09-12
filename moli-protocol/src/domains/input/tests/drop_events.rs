use super::*;

async fn drop_fixture(control: &str) -> TestContext {
    let mut ctx = TestContext::new();
    let html = r#"<html><body style='margin:0'>CONTROL<script>
        const editor = document.getElementById('editor');
        editor.style.cssText = 'position:absolute;left:0;top:0;width:200px;height:100px';
        editor.focus();
        if (!('value' in editor)) {
            const range = document.createRange();
            range.selectNodeContents(editor);
            range.collapse(false);
            getSelection().removeAllRanges();
            getSelection().addRange(range);
        }
        window.__events = [];
        window.__transfers = [];
        window.__cancel = false;
        window.__mutate = false;
        window.__dropTransfer = null;
        const NativeInputEvent = InputEvent;
        editor.addEventListener('drop', event => __dropTransfer = event.dataTransfer);
        for (const type of ['beforeinput', 'input']) {
            editor.addEventListener(type, event => {
                const transfer = event.dataTransfer;
                __events.push({type, native: event instanceof NativeInputEvent,
                    data: event.data, inputType: event.inputType, composing: event.isComposing,
                    trusted: event.isTrusted, bubbles: event.bubbles, composed: event.composed,
                    cancelable: event.cancelable,
                    transfer: transfer ? {text: transfer.getData('text/plain'),
                        html: transfer.getData('text/html'), types: Array.from(transfer.types),
                        effect: transfer.effectAllowed, drop: transfer.dropEffect} : transfer});
                __transfers.push(transfer);
                if (type === 'beforeinput' && __cancel) event.preventDefault();
                if (type === 'beforeinput' && __mutate) {
                    const initialTypes = transfer.types;
                    transfer.setData('text/plain', 'changed');
                    transfer.clearData();
                    transfer.items.clear();
                    transfer.effectAllowed = 'move';
                    const added = transfer.items.add('duplicate', 'text/plain');
                    const addedFile = transfer.items.add(new File(['x'], 'x.txt'));
                    let removed = '';
                    try { transfer.items.remove(0); } catch (error) { removed = error.name; }
                    let conversion = '';
                    try {
                        transfer.items.add({toString() { throw new RangeError('conversion'); }}, 'text/plain');
                    } catch (error) { conversion = error.name; }
                    window.__mutation = {added, addedFile, removed, conversion,
                        sameTypes: initialTypes === transfer.types};
                }
            });
        }
    </script></body></html>"#.replace("CONTROL", control);
    with_loaded_document(&mut ctx, &html).await;
    ctx
}

async fn drop_text(ctx: &mut TestContext, html: bool) {
    let mut items = vec![json!({"mimeType": "text/plain", "data": "hello"})];
    if html {
        items.push(json!({"mimeType": "text/html", "data": "<b>hello</b>"}));
    }
    for event_type in ["dragEnter", "dragOver", "drop"] {
        ctx.process_async(json!({
            "id": 701, "method": "Input.dispatchDragEvent",
            "params": {"type": event_type, "x": 20, "y": 20,
                "data": {"items": items, "dragOperationsMask": 1}}
        }))
        .await;
        ctx.expect_result(701, json!({}), None);
    }
}

fn input_event(
    event_type: &str,
    data: serde_json::Value,
    transfer: serde_json::Value,
) -> serde_json::Value {
    json!({"type": event_type, "native": true, "data": data,
        "inputType": "insertFromDrop", "composing": false, "trusted": true,
        "bubbles": true, "composed": true, "cancelable": event_type == "beforeinput", "transfer": transfer})
}

fn transfer(html: bool) -> serde_json::Value {
    json!({"text": "hello", "html": if html { "<b>hello</b>" } else { "" },
        "types": if html { vec!["text/plain", "text/html"] } else { vec!["text/plain"] },
        "effect": "copy", "drop": "none"})
}

async fn events(ctx: &mut TestContext) -> serde_json::Value {
    serde_json::from_str(&evaluate_string(ctx, "JSON.stringify(__events)").await).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn rich_drop_emits_native_input_events_and_preserves_readonly_transfer() {
    for html in [false, true] {
        let mut ctx = drop_fixture("<div id='editor' contenteditable='true'></div>").await;
        assert!(evaluate_bool(&mut ctx, "(__mutate = true)").await);
        // Native dispatch must not call a page-replaced constructor.
        assert!(
            evaluate_bool(
                &mut ctx,
                "(window.InputEvent = function() { throw new Error('replaced'); }, true)"
            )
            .await
        );
        drop_text(&mut ctx, html).await;
        assert_eq!(
            events(&mut ctx).await,
            json!([
                input_event("beforeinput", json!(null), transfer(html)),
                input_event("input", json!(null), transfer(html)),
            ])
        );
        assert_eq!(
            evaluate_string(&mut ctx, "editor.textContent").await,
            "hello"
        );
        assert_eq!(
            evaluate_bool(&mut ctx, "editor.querySelector('b') !== null").await,
            html
        );
        assert_eq!(
            evaluate_string(&mut ctx, "JSON.stringify(__mutation)").await,
            r#"{"added":null,"addedFile":null,"removed":"InvalidStateError","conversion":"RangeError","sameTypes":true}"#
        );
        assert!(
            evaluate_bool(
                &mut ctx,
                "__transfers[0] === __transfers[1] && __transfers[0] !== __dropTransfer"
            )
            .await
        );
        assert_eq!(
            evaluate_string(
                &mut ctx,
                "(__dropTransfer.clearData(), __transfers[0].getData('text/plain'))"
            )
            .await,
            "hello"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn canceling_drop_beforeinput_preserves_content_and_emits_no_input() {
    let mut ctx = drop_fixture("<div id='editor' contenteditable='true'>base</div>").await;
    assert!(evaluate_bool(&mut ctx, "(__cancel = true)").await);
    drop_text(&mut ctx, true).await;
    assert_eq!(
        events(&mut ctx).await,
        json!([input_event("beforeinput", json!(null), transfer(true)),])
    );
    assert_eq!(evaluate_string(&mut ctx, "editor.innerHTML").await, "base");
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_text_drop_keeps_string_payload_without_inserting_html() {
    for control in [
        "<input id='editor'>",
        "<textarea id='editor'></textarea>",
        "<div id='editor' contenteditable='plaintext-only'></div>",
    ] {
        let mut ctx = drop_fixture(control).await;
        drop_text(&mut ctx, true).await;
        // Chromium exposes text to plaintext-only beforeinput, but retains the
        // transfer on the subsequent editing-command input event.
        let after = if control.contains("plaintext-only") {
            input_event("input", json!(null), transfer(true))
        } else {
            input_event("input", json!("hello"), json!(null))
        };
        assert_eq!(
            events(&mut ctx).await,
            json!([
                input_event("beforeinput", json!("hello"), json!(null)),
                after,
            ])
        );
        assert_eq!(
            evaluate_string(&mut ctx, "editor.value ?? editor.innerHTML").await,
            "hello"
        );
    }
}
