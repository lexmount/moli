use super::*;
use serde_json::Value;

#[tokio::test(flavor = "multi_thread")]
async fn cdp_keyboard_location_preserves_both_modifier_sides_on_keydown_and_keyup() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<input id=field><script>
      window.events=[];
      for (const type of ['keydown','keypress','keyup'])
        document.addEventListener(type,e=>events.push([e.type,e.key,e.code,e.location,e.repeat]));
      field.focus();
    </script>"#,
    )
    .await;
    let mut expected = Vec::new();
    let mut id = 100;
    for key in ["Shift", "Control", "Alt", "Meta"] {
        for (side, location) in [("Left", 1), ("Right", 2)] {
            let code = format!("{key}{side}");
            for (event_type, name) in [("rawKeyDown", "keydown"), ("keyUp", "keyup")] {
                ctx.process_async(json!({"id":id,"method":"Input.dispatchKeyEvent","params":{
                    "type":event_type,"key":key,"code":code,"location":location
                }}))
                .await;
                ctx.expect_result(id, json!({}), None);
                expected.push(json!([name, key, code, location, false]));
                id += 1;
            }
        }
    }
    assert_eq!(
        serde_json::from_str::<Value>(&evaluate_string(&mut ctx, "JSON.stringify(events)").await)
            .unwrap(),
        Value::Array(expected),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_keyboard_location_uses_explicit_location_and_keypad_for_all_character_phases() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<input id=field><script>
      window.events=[];
      for (const type of ['keydown','keypress','keyup'])
        document.addEventListener(type,e=>events.push([e.type,e.key,e.code,e.location,e.repeat]));
      field.focus();
    </script>"#,
    )
    .await;
    for (id, params) in [
        (
            100,
            json!({"type":"rawKeyDown","key":"Shift","code":"ShiftLeft"}),
        ),
        (
            101,
            json!({"type":"keyUp","key":"Shift","code":"ShiftLeft"}),
        ),
        (
            102,
            json!({"type":"keyDown","key":"1","code":"Numpad1","text":"1","isKeypad":true,"autoRepeat":true}),
        ),
        (
            103,
            json!({"type":"keyUp","key":"1","code":"Numpad1","isKeypad":true}),
        ),
        (
            104,
            json!({"type":"char","key":"/","code":"NumpadDivide","text":"/","isKeypad":true,"location":1}),
        ),
        (
            105,
            json!({"type":"rawKeyDown","key":"a","code":"KeyA","location":2}),
        ),
        (
            106,
            json!({"type":"keyUp","key":"a","code":"KeyA","location":2}),
        ),
        (
            107,
            json!({"type":"rawKeyDown","key":"a","code":"KeyA","location":3}),
        ),
        (
            108,
            json!({"type":"rawKeyDown","key":"a","code":"KeyA","location":-1}),
        ),
        (
            109,
            json!({"type":"rawKeyDown","key":"a","code":"KeyA","location":4}),
        ),
        (
            110,
            json!({"type":"rawKeyDown","key":"a","code":"KeyA","location":5}),
        ),
        (111, json!({"type":"rawKeyDown","key":"1","code":"Numpad1"})),
        (112, json!({"type":"keyUp","key":"1","code":"Numpad1"})),
    ] {
        ctx.process_async(json!({"id":id,"method":"Input.dispatchKeyEvent","params":params}))
            .await;
        ctx.expect_result(id, json!({}), None);
    }
    assert_eq!(
        serde_json::from_str::<Value>(&evaluate_string(&mut ctx, "JSON.stringify(events)").await)
            .unwrap(),
        json!([
            ["keydown", "Shift", "ShiftLeft", 0, false],
            ["keyup", "Shift", "ShiftLeft", 0, false],
            ["keydown", "1", "Numpad1", 3, true],
            ["keypress", "1", "Numpad1", 3, true],
            ["keyup", "1", "Numpad1", 3, false],
            ["keypress", "/", "NumpadDivide", 3, false],
            ["keydown", "a", "KeyA", 2, false],
            ["keyup", "a", "KeyA", 2, false],
            ["keydown", "a", "KeyA", 1, false],
            ["keydown", "a", "KeyA", 1, false],
            ["keydown", "a", "KeyA", 0, false],
            ["keydown", "a", "KeyA", 1, false],
            ["keydown", "1", "Numpad1", 0, false],
            ["keyup", "1", "Numpad1", 0, false],
        ]),
    );
    assert_eq!(evaluate_string(&mut ctx, "field.value").await, "1/");
}
