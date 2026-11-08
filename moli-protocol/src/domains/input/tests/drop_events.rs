use super::*;

async fn drop_fixture(control: &str) -> TestContext {
    let mut ctx = TestContext::new();
    let html = r#"<html><head><meta charset='utf-8'></head><body style='margin:0'>CONTROL<script>
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
    drop_payload(ctx, "hello", html.then_some("<b>hello</b>")).await;
}

async fn drop_payload(ctx: &mut TestContext, text: &str, html: Option<&str>) {
    let mut items = vec![json!({"mimeType": "text/plain", "data": text})];
    if let Some(html) = html {
        items.push(json!({"mimeType": "text/html", "data": html}));
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

#[tokio::test(flavor = "multi_thread")]
async fn drop_beforeinput_revalidates_connected_document_and_editability() {
    for mode in ["true", "plaintext-only"] {
        for (mutation, script) in [
            ("remove", "editor.remove();"),
            ("disable", "editor.contentEditable='false';"),
            (
                "adopt",
                "window.__otherDocument=document.implementation.createHTMLDocument('other');window.__otherDocument.body.appendChild(editor);",
            ),
        ] {
            let mut ctx = drop_fixture(&format!(
                "<div id='editor' contenteditable='{mode}'>base</div>"
            ))
            .await;
            assert!(evaluate_bool(&mut ctx, &format!(
                "(() => {{window.__originalDocument=editor.ownerDocument;editor.addEventListener('beforeinput', () => {{{script}}});return true;}})()"
            )).await);
            drop_text(&mut ctx, true).await;
            let before = if mode == "plaintext-only" {
                input_event("beforeinput", json!("hello"), json!(null))
            } else {
                input_event("beforeinput", json!(null), transfer(true))
            };
            assert_eq!(
                events(&mut ctx).await,
                json!([before]),
                "{mode}: {mutation}"
            );
            assert_eq!(
                evaluate_string(&mut ctx, "editor.textContent").await,
                "base",
                "{mode}: {mutation}"
            );
            let target_state = evaluate_string(
                &mut ctx,
                "JSON.stringify([editor.ownerDocument===window.__originalDocument,editor.isConnected,editor.contentEditable])",
            ).await;
            let expected = match mutation {
                "remove" => json!([true, false, mode]),
                "disable" => json!([true, true, "false"]),
                "adopt" => json!([false, true, mode]),
                _ => unreachable!(),
            };
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&target_state).unwrap(),
                expected,
                "{mode}: {mutation}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn contenteditable_drop_and_insert_text_use_utf16_selection_and_caret_offsets() {
    for mode in ["true", "plaintext-only"] {
        for route in ["drop", "insertText"] {
            for (initial, anchor, focus, replacement, expected, caret) in [
                ("😀abc", 2, 3, "X", "😀Xbc", 3),
                ("a😀bc", 1, 3, "🦀", "a🦀bc", 3),
                ("Ae\u{301}B", 1, 3, "👩‍💻", "A👩‍💻B", 6),
                ("A😀B", 1, 3, "e\u{301}", "Ae\u{301}B", 3),
                ("😀abc", 3, 2, "🚀", "😀🚀bc", 4),
                ("😀ab", 2, 2, "𐐀", "😀𐐀ab", 4),
            ] {
                let mut ctx =
                    drop_fixture(&format!("<div id='editor' contenteditable='{mode}'></div>"))
                        .await;
                assert!(evaluate_bool(&mut ctx, &format!(
                    "(() => {{editor.textContent={};const text=editor.firstChild;getSelection().setBaseAndExtent(text,{anchor},text,{focus});return true;}})()",
                    json!(initial),
                )).await);
                if route == "drop" {
                    drop_payload(&mut ctx, replacement, None).await;
                } else {
                    ctx.process_async(json!({
                        "id": 702, "method": "Input.insertText", "params": {"text":replacement}
                    }))
                    .await;
                    ctx.expect_result(702, json!({}), None);
                }
                let result = evaluate_string(
                    &mut ctx,
                    "JSON.stringify([editor.textContent,getSelection().anchorNode===editor.firstChild,getSelection().focusNode===editor.firstChild,getSelection().anchorOffset,getSelection().focusOffset,getSelection().isCollapsed])",
                ).await;
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&result).unwrap(),
                    json!([expected, true, true, caret, caret, true]),
                    "{mode}: {route}: {initial:?} {anchor}..{focus} -> {replacement:?}"
                );
                let expected_events = if route == "drop" {
                    let snapshot = json!({"text":replacement,"html":"","types":["text/plain"],"effect":"copy","drop":"none"});
                    let before = if mode == "plaintext-only" {
                        input_event("beforeinput", json!(replacement), json!(null))
                    } else {
                        input_event("beforeinput", json!(null), snapshot.clone())
                    };
                    json!([before, input_event("input", json!(null), snapshot)])
                } else {
                    let mut before = input_event("beforeinput", json!(replacement), json!(null));
                    before["inputType"] = json!("insertText");
                    let mut after = input_event("input", json!(replacement), json!(null));
                    after["inputType"] = json!("insertText");
                    json!([before, after])
                };
                assert_eq!(
                    events(&mut ctx).await,
                    expected_events,
                    "{mode}: {route}: {initial:?}"
                );
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unicode_drop_keeps_rich_fragment_and_plaintext_insertion_paths_distinct() {
    for mode in ["true", "plaintext-only"] {
        let mut ctx = drop_fixture(&format!(
            "<div id='editor' contenteditable='{mode}'>😀abc</div>"
        ))
        .await;
        assert!(evaluate_bool(&mut ctx,
            "(() => {const text=editor.firstChild;getSelection().setBaseAndExtent(text,2,text,3);return true;})()"
        ).await);
        drop_payload(&mut ctx, "🦀e\u{301}", Some("<b>🦀e\u{301}</b>")).await;
        let result = evaluate_string(
            &mut ctx,
            "JSON.stringify([editor.textContent,editor.querySelector('b')!==null,getSelection().anchorNode===editor,getSelection().anchorNode===editor.firstChild,getSelection().anchorOffset,getSelection().focusOffset,getSelection().isCollapsed])",
        ).await;
        let expected = if mode == "true" {
            json!(["😀🦀e\u{301}bc", true, true, false, 2, 2, true])
        } else {
            json!(["😀🦀e\u{301}bc", false, false, true, 6, 6, true])
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).unwrap(),
            expected,
            "{mode}"
        );
        let snapshot = json!({"text":"🦀e\u{301}","html":"<b>🦀e\u{301}</b>","types":["text/plain","text/html"],"effect":"copy","drop":"none"});
        let before = if mode == "plaintext-only" {
            input_event("beforeinput", json!("🦀e\u{301}"), json!(null))
        } else {
            input_event("beforeinput", json!(null), snapshot.clone())
        };
        assert_eq!(
            events(&mut ctx).await,
            json!([before, input_event("input", json!(null), snapshot)]),
            "{mode}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn drop_beforeinput_switch_to_plaintext_only_preserves_rich_event_payload() {
    let mut ctx = drop_fixture("<div id='editor' contenteditable='true'></div>").await;
    assert!(evaluate_bool(
        &mut ctx,
        "(() => {editor.addEventListener('beforeinput', () => {editor.contentEditable='plaintext-only';});return true;})()",
    ).await);
    drop_text(&mut ctx, true).await;
    assert_eq!(
        events(&mut ctx).await,
        json!([
            input_event("beforeinput", json!(null), transfer(true)),
            input_event("input", json!(null), transfer(true)),
        ]),
    );
    let result = evaluate_string(
        &mut ctx,
        "JSON.stringify([editor.contentEditable,editor.innerHTML,editor.querySelector('b')===null,getSelection().focusNode===editor.firstChild,getSelection().focusOffset,__transfers[0]===__transfers[1]])",
    ).await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        json!(["plaintext-only", "hello", true, true, 5, true]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn beforeinput_revalidates_the_inherited_editing_host_and_input_target() {
    for mode in ["true", "plaintext-only"] {
        for route in ["drop", "insertText"] {
            let mut ctx = drop_fixture(
                "<div id='outer'><div id='editor' contenteditable='true'></div></div>",
            )
            .await;
            assert!(evaluate_bool(&mut ctx, &format!(
                "(() => {{window.__retargetedInputs=[];window.__retargetedTransfer=null;\
                document.addEventListener('input', event => {{\
                    __retargetedInputs.push({{target:event.target.id,native:event instanceof InputEvent,\
                    inputType:event.inputType,data:event.data,\
                    transfer:event.dataTransfer?event.dataTransfer.getData('text/plain'):null}});\
                    __retargetedTransfer=event.dataTransfer;\
                }});\
                editor.addEventListener('beforeinput', () => {{\
                    document.getElementById('outer').contentEditable={};\
                    editor.removeAttribute('contenteditable');\
                }});return true;}})()",
                json!(mode),
            )).await);
            if route == "drop" {
                drop_text(&mut ctx, true).await;
            } else {
                ctx.process_async(json!({
                    "id": 702, "method": "Input.insertText", "params": {"text":"hello"}
                }))
                .await;
                ctx.expect_result(702, json!({}), None);
            }
            let before = if route == "drop" {
                input_event("beforeinput", json!(null), transfer(true))
            } else {
                let mut event = input_event("beforeinput", json!("hello"), json!(null));
                event["inputType"] = json!("insertText");
                event
            };
            // The editor receives beforeinput, but input must target its new host.
            assert_eq!(events(&mut ctx).await, json!([before]), "{mode}: {route}");
            let result = evaluate_string(
                &mut ctx,
                "JSON.stringify([editor.textContent,editor.querySelector('b')!==null,editor.hasAttribute('contenteditable'),editor.isContentEditable,__retargetedInputs,__retargetedTransfer===__transfers[0]])",
            ).await;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&result).unwrap(),
                json!([
                    "hello", mode == "true" && route == "drop", false, true,
                    [{"target":"outer","native":true,
                      "inputType":if route == "drop" {"insertFromDrop"} else {"insertText"},
                      "data":if route == "drop" {json!(null)} else {json!("hello")},
                      "transfer":if route == "drop" {json!("hello")} else {json!(null)}}],
                    true
                ]),
                "{mode}: {route}"
            );
        }
    }
}
