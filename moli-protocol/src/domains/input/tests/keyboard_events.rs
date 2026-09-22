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
