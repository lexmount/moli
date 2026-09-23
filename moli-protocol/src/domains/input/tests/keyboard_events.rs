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
                if (type === 'keypress') {
                    __keypresses.push({key: event.key, code: event.code,
                        keyCode: event.keyCode, charCode: event.charCode,
                        which: event.which, trusted: event.isTrusted});
                }
                if (type === 'beforeinput' || type === 'input') {
                    __inputEvents.push({type, inputEvent: event instanceof InputEvent,
                        data: event.data, inputType: event.inputType,
                        isComposing: event.isComposing, trusted: event.isTrusted,
                        bubbles: event.bubbles, composed: event.composed,
                        cancelable: event.cancelable,
                        value: event.target.value ?? event.target.textContent});
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

async fn assert_input_events(
    ctx: &mut TestContext,
    input_type: &str,
    before_data: Option<&str>,
    input_data: Option<&str>,
    before_value: &str,
    after_value: &str,
    canceled: bool,
) {
    let event = |event_type: &str, data: Option<&str>, value: &str| {
        json!({"type": event_type, "inputEvent": true, "data": data,
            "inputType": input_type, "isComposing": false, "trusted": true,
            "bubbles": true, "composed": true, "cancelable": event_type == "beforeinput",
            "value": value})
    };
    let mut expected = vec![event("beforeinput", before_data, before_value)];
    if !canceled {
        expected.push(event("input", input_data, after_value));
    }
    let actual: serde_json::Value =
        serde_json::from_str(&evaluate_string(ctx, "JSON.stringify(__inputEvents)").await)
            .expect("input event log must be JSON");
    assert_eq!(actual, json!(expected));
    assert_eq!(field_value(ctx).await, after_value);
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_text_editing_dispatches_input_events_with_text_and_cancellation() {
    for control in CONTROLS {
        for route in ["keyDown", "char", "insertText"] {
            for text in ["a", "😀z"] {
                for canceled in [false, true] {
                    let mut ctx = keyboard_fixture(control).await;
                    if canceled {
                        assert!(evaluate_bool(&mut ctx, "(__cancel = 'beforeinput') !== ''").await);
                    }
                    if route == "insertText" {
                        ctx.process_async(json!({"id": 902, "method": "Input.insertText",
                            "params": {"text": text}}))
                            .await;
                        ctx.expect_result(902, json!({}), None);
                    } else {
                        dispatch(&mut ctx, route, text, "", text).await;
                    }
                    assert_input_events(
                        &mut ctx,
                        "insertText",
                        Some(text),
                        Some(text),
                        "",
                        if canceled { "" } else { text },
                        canceled,
                    )
                    .await;
                }
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_text_editing_deletion_input_events_report_direction_and_null_data() {
    for control in CONTROLS {
        for (key, input_type) in [
            ("Backspace", "deleteContentBackward"),
            ("Delete", "deleteContentForward"),
        ] {
            for canceled in [false, true] {
                let mut ctx = keyboard_fixture(control).await;
                assert!(
                    evaluate_bool(
                        &mut ctx,
                        r#"(() => {
                    const field = document.getElementById('field');
                    if ('value' in field) {
                        field.value = 'abc'; field.setSelectionRange(1, 2);
                    } else {
                        field.textContent = 'abc';
                        const range = document.createRange();
                        range.setStart(field.firstChild, 1); range.setEnd(field.firstChild, 2);
                        const selection = getSelection();
                        selection.removeAllRanges(); selection.addRange(range);
                    }
                    return true;
                })()"#
                    )
                    .await
                );
                if canceled {
                    assert!(evaluate_bool(&mut ctx, "(__cancel = 'beforeinput') !== ''").await);
                }
                dispatch(&mut ctx, "rawKeyDown", key, key, "").await;
                assert_input_events(
                    &mut ctx,
                    input_type,
                    None,
                    None,
                    "abc",
                    if canceled { "abc" } else { "ac" },
                    canceled,
                )
                .await;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_textarea_enter_input_events_report_line_break_and_null_data() {
    for (route, key) in [("keyDown", "Enter"), ("char", "")] {
        for canceled in [false, true] {
            let mut ctx = keyboard_fixture(CONTROLS[1]).await;
            if canceled {
                assert!(evaluate_bool(&mut ctx, "(__cancel = 'beforeinput') !== ''").await);
            }
            dispatch(&mut ctx, route, key, "", "\r").await;
            assert_input_events(
                &mut ctx,
                "insertLineBreak",
                None,
                None,
                "",
                if canceled { "" } else { "\n" },
                canceled,
            )
            .await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_text_editing_input_data_reflects_maxlength_and_single_line_normalization() {
    for control in &CONTROLS[..2] {
        for (text, inserted) in [("abc", "ab"), ("😀X", "😀"), ("a\rb", "a ")] {
            if *control == CONTROLS[1] && text.contains('\r') {
                continue;
            }
            let mut ctx = keyboard_fixture(control).await;
            assert!(
                evaluate_bool(
                    &mut ctx,
                    "(document.getElementById('field').maxLength = 2) === 2"
                )
                .await
            );
            dispatch(&mut ctx, "char", "", "", text).await;
            assert_input_events(
                &mut ctx,
                "insertText",
                Some(text),
                Some(inserted),
                "",
                inserted,
                false,
            )
            .await;
        }
    }
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
async fn cdp_canceled_raw_keydown_suppresses_only_its_following_char_phase() {
    let mut ctx = keyboard_fixture(CONTROLS[0]).await;
    assert!(evaluate_bool(&mut ctx, "(__cancel = 'keydown') === 'keydown'").await);

    dispatch(&mut ctx, "rawKeyDown", "a", "KeyA", "").await;
    dispatch(&mut ctx, "char", "a", "KeyA", "a").await;
    assert_eq!(field_value(&mut ctx).await, "");
    assert_eq!(events(&mut ctx).await, json!(["keydown:field"]));

    assert!(evaluate_bool(&mut ctx, "(__cancel = '') === ''").await);
    dispatch(&mut ctx, "rawKeyDown", "b", "KeyB", "").await;
    dispatch(&mut ctx, "char", "b", "KeyB", "b").await;
    assert_eq!(field_value(&mut ctx).await, "b");
    assert_eq!(
        events(&mut ctx).await,
        json!([
            "keydown:field",
            "keydown:field",
            "keypress:field",
            "beforeinput:field",
            "input:field"
        ])
    );

    assert!(evaluate_bool(&mut ctx, "(__cancel = 'keydown') === 'keydown'").await);
    dispatch(&mut ctx, "rawKeyDown", "c", "KeyC", "").await;
    dispatch(&mut ctx, "keyUp", "c", "KeyC", "").await;
    assert!(evaluate_bool(&mut ctx, "(__cancel = '') === ''").await);
    dispatch(&mut ctx, "char", "c", "KeyC", "c").await;
    assert_eq!(field_value(&mut ctx).await, "bc");
    assert_eq!(
        events(&mut ctx).await,
        json!([
            "keydown:field",
            "keydown:field",
            "keypress:field",
            "beforeinput:field",
            "input:field",
            "keydown:field",
            "keyup:field",
            "keypress:field",
            "beforeinput:field",
            "input:field"
        ])
    );
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

#[tokio::test(flavor = "multi_thread")]
async fn cdp_deletion_observes_live_selection_after_beforeinput_without_temporary_selection() {
    // Chromium 145: beforeinput sees the original caret. An uncanceled edit
    // uses listener changes; cancellation preserves them without a rollback.
    for control in &CONTROLS[..2] {
        for (effect, canceled_value, canceled_selection, backward, forward, editable) in [
            ("", "abc", [1, 1], ("bc", [0, 0]), ("ac", [1, 1]), true),
            (
                "field.setSelectionRange(2,2)",
                "abc",
                [2, 2],
                ("ac", [1, 1]),
                ("ab", [2, 2]),
                true,
            ),
            (
                "field.setSelectionRange(0,2)",
                "abc",
                [0, 2],
                ("c", [0, 0]),
                ("c", [0, 0]),
                true,
            ),
            (
                "field.value='wxyz';field.setSelectionRange(2,2)",
                "wxyz",
                [2, 2],
                ("wyz", [1, 1]),
                ("wxz", [2, 2]),
                true,
            ),
            (
                "field.readOnly=true",
                "abc",
                [1, 1],
                ("abc", [1, 1]),
                ("abc", [1, 1]),
                false,
            ),
            (
                "field.remove()",
                "abc",
                [1, 1],
                ("abc", [1, 1]),
                ("abc", [1, 1]),
                false,
            ),
        ] {
            for (key, uncanceled) in [("Backspace", backward), ("Delete", forward)] {
                for cancel in [false, true] {
                    let mut ctx = keyboard_fixture(control).await;
                    assert!(evaluate_bool(&mut ctx, &format!(r#"(() => {{
                        const field = document.getElementById('field');
                        window.__field = field; window.__selectionEvents = [];
                        field.value='abc'; field.setSelectionRange(1,1);
                        for (const type of ['beforeinput', 'input']) field.addEventListener(type, e => {{
                            __selectionEvents.push([type,field.selectionStart,field.selectionEnd]);
                            if(type === 'beforeinput') {{ {effect}; if({cancel}) e.preventDefault(); }}
                        }});
                        return true;
                    }})()"#)).await);
                    dispatch(&mut ctx, "rawKeyDown", key, key, "").await;
                    let (value, selection) = if cancel {
                        (canceled_value, canceled_selection)
                    } else {
                        uncanceled
                    };
                    let mut expected_events = vec![json!(["beforeinput", 1, 1])];
                    if !cancel && editable {
                        expected_events.push(json!(["input", selection[0], selection[1]]));
                    }
                    let actual: serde_json::Value = serde_json::from_str(&evaluate_string(&mut ctx,
                        "JSON.stringify([__field.value,[__field.selectionStart,__field.selectionEnd],__selectionEvents])").await).unwrap();
                    assert_eq!(
                        actual,
                        json!([value, selection, expected_events]),
                        "{control} {key} {effect} cancel={cancel}"
                    );
                }
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_deletion_at_text_boundary_still_dispatches_beforeinput_without_editing() {
    for control in &CONTROLS[..2] {
        for (key, caret) in [("Backspace", 0), ("Delete", 3)] {
            let mut ctx = keyboard_fixture(control).await;
            assert!(evaluate_bool(&mut ctx, &format!("(()=>{{const field=document.getElementById('field');field.value='abc';field.setSelectionRange({caret},{caret});return true}})()")).await);
            dispatch(&mut ctx, "rawKeyDown", key, key, "").await;
            assert_eq!(
                events(&mut ctx).await,
                json!(["keydown:field", "beforeinput:field"])
            );
            assert_eq!(field_value(&mut ctx).await, "abc");
        }
    }
}
