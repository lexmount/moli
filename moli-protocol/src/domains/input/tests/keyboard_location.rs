use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn cdp_keyboard_location_reaches_each_key_phase_and_focused_document() {
    for child in [false, true] {
        let mut ctx = TestContext::new();
        with_loaded_document(&mut ctx, "<!doctype html><body><input id=field></body>").await;
        assert!(evaluate_bool(&mut ctx, &format!(r#"(() => {{
            const frame = {child} ? document.body.appendChild(document.createElement('iframe')) : null;
            const doc = frame ? frame.contentDocument : document;
            if (frame) doc.body.innerHTML = '<input id=field>';
            globalThis.__locationEvents = [];
            for (const type of ['keydown', 'keypress', 'keyup']) doc.addEventListener(type, e => __locationEvents.push({{type:e.type,location:e.location,target:e.target.id,trusted:e.isTrusted}}));
            doc.getElementById('field').focus();
            return true;
        }})()"#)).await);
        for (extra, location) in [
            (json!({}), 0),
            (json!({"location":0}), 0),
            (json!({"location":1}), 1),
            (json!({"location":2}), 2),
            (json!({"location":3}), 1),
            (json!({"location":-1}), 1),
            (json!({"location":100}), 0),
            (json!({"isKeypad":true}), 3),
            (json!({"isKeypad":true,"location":1}), 3),
            (json!({"isKeypad":true,"location":2}), 3),
        ] {
            for event_type in ["rawKeyDown", "keyDown", "char", "keyUp"] {
                let mut params = json!({"type":event_type,"key":"x","code":"ShiftLeft","text":"x"});
                params
                    .as_object_mut()
                    .unwrap()
                    .extend(extra.as_object().unwrap().clone());
                ctx.process_async(
                    json!({"id":9901,"method":"Input.dispatchKeyEvent","params":params}),
                )
                .await;
                ctx.expect_result(9901, json!({}), None);
                let actual: serde_json::Value = serde_json::from_str(
                    &evaluate_string(&mut ctx, "JSON.stringify(__locationEvents.splice(0))").await,
                )
                .unwrap();
                let names = match event_type {
                    "rawKeyDown" => vec!["keydown"],
                    "keyDown" => vec!["keydown", "keypress"],
                    "char" => vec!["keypress"],
                    "keyUp" => vec!["keyup"],
                    _ => unreachable!(),
                };
                let expected = names.into_iter().map(|name|json!({"type":name,"location":location,"target":"field","trusted":true})).collect::<Vec<_>>();
                assert_eq!(
                    actual,
                    json!(expected),
                    "{extra} {event_type} child={child}"
                );
            }
        }
    }
}
