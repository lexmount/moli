use super::*;

const CONTROLS: [&str; 3] = [
    "<input id='field'>",
    "<textarea id='field'></textarea>",
    "<div id='field' contenteditable='true'></div>",
];

async fn keyboard_fixture(control: &str) -> TestContext {
    let mut ctx = TestContext::new();
    let html = r#"<html><body>CONTROL<input id='other'><script>
        window.__events = [];
        window.__keypresses = [];
        window.__inputEvents = [];
        window.__cancel = '';
        window.__moveFocus = false;
        for (const type of ['keydown', 'keypress', 'beforeinput', 'input', 'keyup']) {
            document.addEventListener(type, event => {
                __events.push(event.type + ':' + event.target.id);
                if (type === 'beforeinput' || type === 'input') {
                    __inputEvents.push({type, data: event.data, inputType: event.inputType,
                        isComposing: event.isComposing, native: event instanceof InputEvent,
                        ui: event instanceof UIEvent, trusted: event.isTrusted,
                        bubbles: event.bubbles, cancelable: event.cancelable, composed: event.composed});
                }
                if (type === 'keypress') {
                    __keypresses.push({key: event.key, code: event.code,
                        keyCode: event.keyCode, charCode: event.charCode,
                        which: event.which, trusted: event.isTrusted});
                }
                if (type === __cancel) event.preventDefault();
                if (type === 'keydown' && __moveFocus)
                    document.getElementById('other').focus();
            });
        }
        document.getElementById('field').focus();
    </script></body></html>"#
        .replace("CONTROL", control);
    with_loaded_document(&mut ctx, &html).await;
    ctx
}

async fn dispatch(ctx: &mut TestContext, event_type: &str, key: &str, code: &str, text: &str) {
    ctx.process_async(json!({
        "id": 901,
        "method": "Input.dispatchKeyEvent",
        "params": {"type": event_type, "key": key, "code": code, "text": text}
    }))
    .await;
    ctx.expect_result(901, json!({}), None);
}

async fn field_value(ctx: &mut TestContext) -> String {
    evaluate_string(
        ctx,
        "document.getElementById('field').value ?? document.getElementById('field').textContent",
    )
    .await
}

async fn events(ctx: &mut TestContext) -> serde_json::Value {
    serde_json::from_str(&evaluate_string(ctx, "JSON.stringify(__events)").await)
        .expect("event log must be JSON")
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_keyboard_event_types_preserve_character_dispatch_and_editing_order() {
    for control in CONTROLS {
        for (event_type, text, expected_value, expected_events) in [
            (
                "keyDown",
                "a",
                "a",
                vec!["keydown", "keypress", "beforeinput", "input", "keyup"],
            ),
            ("rawKeyDown", "a", "", vec!["keydown", "keyup"]),
            (
                "char",
                "a",
                "a",
                vec!["keypress", "beforeinput", "input", "keyup"],
            ),
            ("keyDown", "", "", vec!["keydown", "keyup"]),
        ] {
            let mut ctx = keyboard_fixture(control).await;
            dispatch(&mut ctx, event_type, "a", "KeyA", text).await;
            dispatch(&mut ctx, "keyUp", "a", "KeyA", "").await;
            assert_eq!(
                field_value(&mut ctx).await,
                expected_value,
                "{control}: {event_type}"
            );
            assert_eq!(
                events(&mut ctx).await,
                json!(
                    expected_events
                        .iter()
                        .map(|event| format!("{event}:field"))
                        .collect::<Vec<_>>()
                ),
                "{control}: {event_type}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_combined_keydown_honors_each_cancellation_boundary() {
    for control in CONTROLS {
        for (cancel, expected) in [
            ("keydown", vec!["keydown", "keyup"]),
            ("keypress", vec!["keydown", "keypress", "keyup"]),
            (
                "beforeinput",
                vec!["keydown", "keypress", "beforeinput", "keyup"],
            ),
        ] {
            let mut ctx = keyboard_fixture(control).await;
            assert!(
                evaluate_bool(&mut ctx, &format!("(__cancel = '{cancel}') === '{cancel}'")).await
            );
            dispatch(&mut ctx, "keyDown", "a", "KeyA", "a").await;
            dispatch(&mut ctx, "keyUp", "a", "KeyA", "").await;
            assert_eq!(
                field_value(&mut ctx).await,
                "",
                "{control}: cancel {cancel}"
            );
            assert_eq!(
                events(&mut ctx).await,
                json!(
                    expected
                        .iter()
                        .map(|event| format!("{event}:field"))
                        .collect::<Vec<_>>()
                ),
                "{control}: cancel {cancel}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_combined_keydown_delivers_trusted_keypresses_for_typed_characters() {
    let mut ctx = keyboard_fixture(CONTROLS[0]).await;
    for (key, code) in [("a", "KeyA"), ("@", "Digit2"), ("A", "KeyA")] {
        dispatch(&mut ctx, "keyDown", key, code, key).await;
        dispatch(&mut ctx, "keyUp", key, code, "").await;
    }
    assert_eq!(field_value(&mut ctx).await, "a@A");
    let observed: serde_json::Value =
        serde_json::from_str(&evaluate_string(&mut ctx, "JSON.stringify(__keypresses)").await)
            .expect("keypress log must be JSON");
    assert_eq!(
        observed,
        json!([
            {"key":"a", "code":"KeyA", "keyCode":97, "charCode":97, "which":97, "trusted":true},
            {"key":"@", "code":"Digit2", "keyCode":64, "charCode":64, "which":64, "trusted":true},
            {"key":"A", "code":"KeyA", "keyCode":65, "charCode":65, "which":65, "trusted":true}
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_combined_keydown_refetches_focus_before_keypress() {
    let mut ctx = keyboard_fixture(CONTROLS[0]).await;
    assert!(evaluate_bool(&mut ctx, "__moveFocus = true").await);
    dispatch(&mut ctx, "keyDown", "a", "KeyA", "a").await;
    dispatch(&mut ctx, "keyUp", "a", "KeyA", "").await;
    assert_eq!(field_value(&mut ctx).await, "");
    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('other').value").await,
        "a"
    );
    assert_eq!(
        events(&mut ctx).await,
        json!([
            "keydown:field",
            "keypress:other",
            "beforeinput:other",
            "input:other",
            "keyup:other"
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_insert_text_does_not_manufacture_keyboard_events() {
    for control in CONTROLS {
        let mut ctx = keyboard_fixture(control).await;
        ctx.process_async(json!({
            "id": 902, "method": "Input.insertText", "params": {"text": "a"}
        }))
        .await;
        ctx.expect_result(902, json!({}), None);
        assert_eq!(field_value(&mut ctx).await, "a");
        assert_eq!(
            events(&mut ctx).await,
            json!(["beforeinput:field", "input:field"])
        );
    }
}

fn expected_input_event(
    event_type: &str,
    input_type: &str,
    data: serde_json::Value,
) -> serde_json::Value {
    json!({"type": event_type, "data": data, "inputType": input_type,
        "isComposing": false, "native": true, "ui": true, "trusted": true,
        "bubbles": true, "cancelable": event_type == "beforeinput", "composed": true})
}

async fn input_events(ctx: &mut TestContext) -> serde_json::Value {
    serde_json::from_str(&evaluate_string(ctx, "JSON.stringify(__inputEvents)").await)
        .expect("input event log must be JSON")
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_text_edits_emit_input_event_payloads_for_all_editable_targets() {
    for control in CONTROLS {
        for command in ["keyDown", "char", "insertText"] {
            let mut ctx = keyboard_fixture(control).await;
            if command == "insertText" {
                ctx.process_async(
                    json!({"id": 902, "method":"Input.insertText", "params":{"text":"hello"}}),
                )
                .await;
                ctx.expect_result(902, json!({}), None);
            } else {
                dispatch(&mut ctx, command, "h", "KeyH", "hello").await;
            }
            assert_eq!(field_value(&mut ctx).await, "hello", "{control}: {command}");
            assert_eq!(
                input_events(&mut ctx).await,
                json!([
                    expected_input_event("beforeinput", "insertText", json!("hello")),
                    expected_input_event("input", "insertText", json!("hello")),
                ]),
                "{control}: {command}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_text_deletions_have_direction_and_null_data() {
    for control in &CONTROLS[..2] {
        for (key, input_type, expected) in [
            ("Backspace", "deleteContentBackward", "ac"),
            ("Delete", "deleteContentForward", "ab"),
        ] {
            let mut ctx = keyboard_fixture(control).await;
            evaluate_string(
                &mut ctx,
                "(() => {field.value='abc'; field.setSelectionRange(2,2); return '';})()",
            )
            .await;
            dispatch(&mut ctx, "rawKeyDown", key, key, "").await;
            assert_eq!(field_value(&mut ctx).await, expected);
            assert_eq!(
                input_events(&mut ctx).await,
                json!([
                    expected_input_event("beforeinput", input_type, json!(null)),
                    expected_input_event("input", input_type, json!(null)),
                ])
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_canceling_beforeinput_deletion_preserves_value_and_selection() {
    for key in ["Backspace", "Delete"] {
        let mut ctx = keyboard_fixture(CONTROLS[0]).await;
        evaluate_string(&mut ctx, "(() => {field.value='abc'; field.setSelectionRange(1,1); __cancel='beforeinput'; return '';})()").await;
        dispatch(&mut ctx, "rawKeyDown", key, key, "").await;
        assert_eq!(
            evaluate_string(
                &mut ctx,
                "JSON.stringify([field.value,field.selectionStart,field.selectionEnd])"
            )
            .await,
            r#"["abc",1,1]"#
        );
        assert_eq!(input_events(&mut ctx).await.as_array().unwrap().len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_textarea_enter_emits_line_break_with_null_data() {
    for (command, text) in [("keyDown", "\r"), ("rawKeyDown", "")] {
        let mut ctx = keyboard_fixture(CONTROLS[1]).await;
        dispatch(&mut ctx, command, "Enter", "Enter", text).await;
        assert_eq!(field_value(&mut ctx).await, "\n");
        assert_eq!(
            input_events(&mut ctx).await,
            json!([
                expected_input_event("beforeinput", "insertLineBreak", json!(null)),
                expected_input_event("input", "insertLineBreak", json!(null)),
            ])
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_text_input_uses_intrinsic_constructor_and_keeps_change_a_plain_event() {
    let mut ctx = keyboard_fixture(CONTROLS[0]).await;
    evaluate_string(
        &mut ctx,
        r#"(() => {
        const NativeInputEvent = InputEvent;
        window.__nativeEvents=[];
        for (const type of ['beforeinput','input','change']) field.addEventListener(type, e =>
            __nativeEvents.push([e.type, e instanceof NativeInputEvent, e.isTrusted]));
        window.InputEvent=function() {throw Error('page constructor must not run');};
        // The general fixture's instanceof uses the replaced constructor; this
        // test inspects the captured intrinsic instead.
        return '';
    })()"#,
    )
    .await;
    dispatch(&mut ctx, "keyDown", "a", "KeyA", "a").await;
    evaluate_string(&mut ctx, "(other.focus(), '')").await;
    assert_eq!(field_value(&mut ctx).await, "a");
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(__nativeEvents)").await,
        r#"[["beforeinput",true,true],["input",true,true],["change",false,true]]"#
    );
}
