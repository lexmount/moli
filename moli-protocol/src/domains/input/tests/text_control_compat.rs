use super::*;

const TEXT_CONTROLS: [&str; 2] = [
    "<input id='field' type='text'>",
    "<textarea id='field'></textarea>",
];

async fn text_control_fixture(control: &str) -> TestContext {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        &format!("<!doctype html><html><body>{control}</body></html>"),
    )
    .await;
    ctx
}

#[tokio::test(flavor = "multi_thread")]
async fn selection_and_set_range_text_offsets_use_utf16_code_units() {
    for control in TEXT_CONTROLS {
        let mut ctx = text_control_fixture(control).await;
        let result = evaluate_string(
            &mut ctx,
            r#"(() => {
                const field = document.getElementById('field');
                field.value = '😀X';
                const length = field.value.length;
                field.setSelectionRange(3, 3);
                const endSelection = [field.selectionStart, field.selectionEnd];
                field.setRangeText('Y', 2, 2, 'end');
                return JSON.stringify({
                    length,
                    endSelection,
                    value: field.value,
                    selection: [field.selectionStart, field.selectionEnd],
                });
            })()"#,
        )
        .await;
        let actual: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            actual,
            json!({
                "length": 3,
                "endSelection": [3, 3],
                "value": "😀YX",
                "selection": [3, 3],
            }),
            "{control}: selection and replacement must count UTF-16 code units"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn set_range_text_rejects_reversed_out_of_bounds_offsets_without_mutation() {
    for control in TEXT_CONTROLS {
        let mut ctx = text_control_fixture(control).await;
        let result = evaluate_string(
            &mut ctx,
            r#"(() => {
                const field = document.getElementById('field');
                field.value = 'abc';
                field.setSelectionRange(1, 2, 'backward');
                let exception = null;
                try {
                    field.setRangeText('Y', 100, 99);
                } catch (error) {
                    exception = {
                        name: error.name,
                        code: error.code,
                        isDOMException: error instanceof DOMException,
                    };
                }
                return JSON.stringify({
                    exception,
                    value: field.value,
                    selection: [field.selectionStart, field.selectionEnd, field.selectionDirection],
                });
            })()"#,
        )
        .await;
        let actual: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            actual,
            json!({
                "exception": {"name": "IndexSizeError", "code": 1, "isDOMException": true},
                "value": "abc",
                "selection": [1, 2, "backward"],
            }),
            "{control}: reversed offsets must throw before clamping or changing the control"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn keyboard_typing_dispatches_trusted_input_events() {
    for control in TEXT_CONTROLS {
        let mut ctx = text_control_fixture(control).await;
        assert!(
            evaluate_bool(
                &mut ctx,
                r#"(() => {
                    const field = document.getElementById('field');
                    window.__textControlEvents = [];
                    for (const type of ['beforeinput', 'input']) {
                        field.addEventListener(type, event => {
                            __textControlEvents.push({
                                type: event.type,
                                constructor: event.constructor.name,
                                isInputEvent: event instanceof InputEvent,
                                data: event.data,
                                inputType: event.inputType,
                                isComposing: event.isComposing,
                                isTrusted: event.isTrusted,
                                bubbles: event.bubbles,
                                cancelable: event.cancelable,
                                composed: event.composed,
                                valueAtDispatch: field.value,
                            });
                        });
                    }
                    field.focus();
                    return document.activeElement === field;
                })()"#,
            )
            .await,
            "{control}: text control must be focused before typing"
        );

        for (id, event_type, text) in [(1, "keyDown", "a"), (2, "keyUp", "")] {
            ctx.process_async(json!({
                "id": id,
                "method": "Input.dispatchKeyEvent",
                "params": {"type": event_type, "key": "a", "code": "KeyA", "text": text},
            }))
            .await;
            ctx.expect_result(id, json!({}), None);
        }

        let result = evaluate_string(
            &mut ctx,
            "JSON.stringify({value: document.getElementById('field').value, events: __textControlEvents})",
        )
        .await;
        let actual: serde_json::Value = serde_json::from_str(&result).unwrap();
        let event = |event_type: &str, cancelable: bool, value: &str| {
            json!({
                "type": event_type,
                "constructor": "InputEvent",
                "isInputEvent": true,
                "data": "a",
                "inputType": "insertText",
                "isComposing": false,
                "isTrusted": true,
                "bubbles": true,
                "cancelable": cancelable,
                "composed": true,
                "valueAtDispatch": value,
            })
        };
        assert_eq!(
            actual,
            json!({
                "value": "a",
                "events": [event("beforeinput", true, ""), event("input", false, "a")],
            }),
            "{control}: native InputEvents must bracket the value change and describe the inserted text"
        );
    }
}
