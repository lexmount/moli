use super::*;

#[tokio::test]
async fn websocket_bidi_input_perform_and_release_actions_use_shared_input() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let page = "data:text/html,<input id='field'><script>window.__events=[];const field=document.getElementById('field');field.focus();field.addEventListener('keydown',event=>window.__events.push(event.type+':'+event.key));field.addEventListener('keyup',event=>window.__events.push(event.type+':'+event.key));</script>";
    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": page,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));
    let captured = send_bidi_command_response(
        &mut socket,
        90,
        "browsingContext.captureScreenshot",
        json!({"context": context_id.clone()}),
    )
    .await;
    assert_eq!(captured["type"], "success");

    let type_a = send_bidi_command_response(
        &mut socket,
        4,
        "input.performActions",
        json!({
            "context": context_id.clone(),
            "actions": [{
                "type": "key",
                "id": "keyboard",
                "actions": [
                    { "type": "keyDown", "value": "a" },
                    { "type": "keyUp", "value": "a" }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(type_a["type"], json!("success"));
    assert_eq!(type_a["result"], json!({}));

    let events_after_a = send_bidi_command_response(
        &mut socket,
        5,
        "script.evaluate",
        json!({
            "expression": "window.__events.join(',')",
            "target": {
                "context": context_id.clone()
            },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(events_after_a["type"], json!("success"));
    assert_eq!(
        events_after_a["result"]["result"]["value"],
        json!("keydown:a,keyup:a")
    );

    let hold_b = send_bidi_command_response(
        &mut socket,
        6,
        "input.performActions",
        json!({
            "context": context_id.clone(),
            "actions": [{
                "type": "key",
                "id": "keyboard",
                "actions": [
                    { "type": "keyDown", "value": "b" }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(hold_b["type"], json!("success"));
    assert_eq!(hold_b["result"], json!({}));

    let release = send_bidi_command_response(
        &mut socket,
        7,
        "input.releaseActions",
        json!({
            "context": context_id.clone()
        }),
    )
    .await;
    assert_eq!(release["type"], json!("success"));
    assert_eq!(release["result"], json!({}));

    let events = send_bidi_command_response(
        &mut socket,
        8,
        "script.evaluate",
        json!({
            "expression": "window.__events.join(',')",
            "target": {
                "context": context_id.clone()
            },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(events["type"], json!("success"));
    assert_eq!(
        events["result"]["result"]["value"],
        json!("keydown:a,keyup:a,keydown:b,keyup:b")
    );

    let invalid_context = send_bidi_command_response(
        &mut socket,
        9,
        "input.performActions",
        json!({
            "context": "missing-context",
            "actions": [{
                "type": "key",
                "id": "keyboard",
                "actions": [{ "type": "keyDown", "value": "x" }]
            }]
        }),
    )
    .await;
    assert_bidi_error(
        &invalid_context,
        "no such frame",
        "input.performActions should reject missing context",
    );

    let invalid_release_context = send_bidi_command_response(
        &mut socket,
        10,
        "input.releaseActions",
        json!({
            "context": "missing-context"
        }),
    )
    .await;
    assert_bidi_error(
        &invalid_release_context,
        "no such frame",
        "input.releaseActions should reject missing context",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_input_set_files_updates_file_input() {
    let first_file = TempPath::new("bidi-set-files-first");
    let second_file = TempPath::new("bidi-set-files-second");
    fs::write(&first_file.path, b"alpha").expect("write first upload file");
    fs::write(&second_file.path, b"bravo!").expect("write second upload file");
    let first_name = first_file
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("first file should have a filename")
        .to_owned();
    let second_name = second_file
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("second file should have a filename")
        .to_owned();

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let page = "data:text/html,<input id='file' type='file' multiple><input id='single' type='file'><script>window.__events=[];for(const id of ['file','single']){const el=document.getElementById(id);for(const type of ['input','change','cancel']){el.addEventListener(type,()=>window.__events.push(id+':'+type+':'+el.files.length));}}</script>";
    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": page,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let file_input = send_bidi_command_response(
        &mut socket,
        4,
        "script.evaluate",
        json!({
            "expression": "document.getElementById('file')",
            "target": { "context": context_id.clone() },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(file_input["type"], json!("success"));
    let file_shared_id = file_input["result"]["result"]["sharedId"]
        .as_str()
        .expect("file input remote value should include sharedId")
        .to_owned();

    for (id, params, expected_error, context) in [
        (
            40,
            json!({
                "context": context_id.clone(),
                "element": null,
                "files": []
            }),
            "invalid argument",
            "input.setFiles should reject non-object element",
        ),
        (
            41,
            json!({
                "context": context_id.clone(),
                "element": { "sharedId": file_shared_id.clone() },
                "files": false
            }),
            "invalid argument",
            "input.setFiles should reject non-array files",
        ),
        (
            42,
            json!({
                "context": context_id.clone(),
                "element": { "sharedId": file_shared_id.clone() },
                "files": [false]
            }),
            "invalid argument",
            "input.setFiles should reject non-string file entries",
        ),
    ] {
        let invalid = send_bidi_command_response(&mut socket, id, "input.setFiles", params).await;
        assert_bidi_error(&invalid, expected_error, context);
    }

    let set_files = send_bidi_command_response(
        &mut socket,
        5,
        "input.setFiles",
        json!({
            "context": context_id.clone(),
            "element": { "sharedId": file_shared_id },
            "files": [
                first_file.path.to_string_lossy().to_string(),
                second_file.path.to_string_lossy().to_string()
            ]
        }),
    )
    .await;
    assert_eq!(
        set_files["type"],
        json!("success"),
        "input.setFiles response: {set_files:?}"
    );
    assert_eq!(set_files["result"], json!({}));

    let summary = send_bidi_command_response(
        &mut socket,
        6,
        "script.evaluate",
        json!({
            "expression": "(()=>{const input=document.getElementById('file');return JSON.stringify({length:input.files.length,names:Array.from(input.files).map(file=>file.name).join('|'),sizes:Array.from(input.files).map(file=>file.size).join('|'),value:input.value,events:window.__events.join(',')});})()",
            "target": { "context": context_id.clone() },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(summary["type"], json!("success"));
    let summary: serde_json::Value = serde_json::from_str(
        summary["result"]["result"]["value"]
            .as_str()
            .expect("summary should be a JSON string"),
    )
    .expect("summary should parse");
    assert_eq!(summary["length"], json!(2));
    assert_eq!(
        summary["names"],
        json!(format!("{first_name}|{second_name}"))
    );
    assert_eq!(summary["sizes"], json!("5|6"));
    assert_eq!(
        summary["value"],
        json!(format!("C:\\fakepath\\{first_name}"))
    );
    assert_eq!(summary["events"], json!("file:input:2,file:change:2"));

    let clear = send_bidi_command_response(
        &mut socket,
        7,
        "input.setFiles",
        json!({
            "context": context_id.clone(),
            "element": { "sharedId": file_shared_id },
            "files": []
        }),
    )
    .await;
    assert_eq!(clear["type"], json!("success"));

    let after_clear = send_bidi_command_response(
        &mut socket,
        8,
        "script.evaluate",
        json!({
            "expression": "(()=>{const input=document.getElementById('file');return JSON.stringify({length:input.files.length,value:input.value,events:window.__events.join(',')});})()",
            "target": { "context": context_id.clone() },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(after_clear["type"], json!("success"));
    let after_clear: serde_json::Value = serde_json::from_str(
        after_clear["result"]["result"]["value"]
            .as_str()
            .expect("clear summary should be a JSON string"),
    )
    .expect("clear summary should parse");
    assert_eq!(after_clear["length"], json!(0));
    assert_eq!(after_clear["value"], json!(""));
    assert_eq!(
        after_clear["events"],
        json!("file:input:2,file:change:2,file:input:0,file:change:0")
    );

    let single_input = send_bidi_command_response(
        &mut socket,
        9,
        "script.evaluate",
        json!({
            "expression": "document.getElementById('single')",
            "target": { "context": context_id.clone() },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(single_input["type"], json!("success"));
    let single_shared_id = single_input["result"]["result"]["sharedId"]
        .as_str()
        .expect("single file input remote value should include sharedId")
        .to_owned();
    let non_multiple = send_bidi_command_response(
        &mut socket,
        10,
        "input.setFiles",
        json!({
            "context": context_id.clone(),
            "element": { "sharedId": single_shared_id },
            "files": [
                first_file.path.to_string_lossy().to_string(),
                second_file.path.to_string_lossy().to_string()
            ]
        }),
    )
    .await;
    assert_bidi_error(
        &non_multiple,
        "unable to set file input",
        "input.setFiles should reject multiple files for non-multiple input",
    );

    let set_single = send_bidi_command_response(
        &mut socket,
        11,
        "input.setFiles",
        json!({
            "context": context_id.clone(),
            "element": { "sharedId": single_shared_id },
            "files": [
                first_file.path.to_string_lossy().to_string()
            ]
        }),
    )
    .await;
    assert_eq!(
        set_single["type"],
        json!("success"),
        "single input.setFiles response: {set_single:?}"
    );

    let set_single_again = send_bidi_command_response(
        &mut socket,
        12,
        "input.setFiles",
        json!({
            "context": context_id.clone(),
            "element": { "sharedId": single_shared_id },
            "files": [
                first_file.path.to_string_lossy().to_string()
            ]
        }),
    )
    .await;
    assert_eq!(
        set_single_again["type"],
        json!("success"),
        "same single input.setFiles response: {set_single_again:?}"
    );

    let after_same_single = send_bidi_command_response(
        &mut socket,
        13,
        "script.evaluate",
        json!({
            "expression": "(()=>{const input=document.getElementById('single');return JSON.stringify({length:input.files.length,names:Array.from(input.files).map(file=>file.name).join('|'),value:input.value,events:window.__events.join(',')});})()",
            "target": { "context": context_id.clone() },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(after_same_single["type"], json!("success"));
    let after_same_single: serde_json::Value = serde_json::from_str(
        after_same_single["result"]["result"]["value"]
            .as_str()
            .expect("same-file summary should be a JSON string"),
    )
    .expect("same-file summary should parse");
    assert_eq!(after_same_single["length"], json!(1));
    assert_eq!(after_same_single["names"], json!(first_name));
    assert_eq!(
        after_same_single["value"],
        json!(format!("C:\\fakepath\\{first_name}"))
    );
    assert_eq!(
        after_same_single["events"],
        json!(
            "file:input:2,file:change:2,file:input:0,file:change:0,single:input:1,single:change:1,single:cancel:1"
        )
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_input_file_dialog_opened_event_matches_wpt_shape() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let subscribe = send_bidi_command_response(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["input.fileDialogOpened"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(
        subscribe["type"],
        json!("success"),
        "session.subscribe should accept input.fileDialogOpened: {subscribe:?}"
    );

    let navigate = send_bidi_command_response(
        &mut socket,
        4,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": "data:text/html,<input id=input type=file multiple />",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "input.click(); input",
                    "target": { "context": context_id.clone() },
                    "awaitPromise": false,
                    "userActivation": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate that opens file dialog");
    let mut messages = recv_until_id(&mut socket, 5).await;
    if bidi_events_by_method(&messages, "input.fileDialogOpened").is_empty() {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == json!("input.fileDialogOpened")
            })
            .await,
        );
    }
    let evaluate = bidi_message_by_id(&messages, 5);
    assert_eq!(
        evaluate["type"],
        json!("success"),
        "script.evaluate should return the clicked input: {evaluate:?}"
    );
    let returned_shared_id = evaluate["result"]["result"]["sharedId"]
        .as_str()
        .expect("returned input should include a sharedId");
    let event = bidi_events_by_method(&messages, "input.fileDialogOpened")
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("input.fileDialogOpened event should arrive: {messages:#?}"));
    assert_eq!(event["type"], json!("event"));
    assert_eq!(event["params"]["context"], json!(context_id));
    assert_eq!(event["params"]["multiple"], json!(true));
    assert_eq!(
        event["params"]["element"]["sharedId"],
        json!(returned_shared_id)
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_input_element_origin_uses_real_geometry() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let page = "data:text/html,<script>window.__clicked=false;window.__wheel=null;document.addEventListener('wheel',event=>{window.__wheel={type:event.type,deltaX:event.deltaX,deltaY:event.deltaY,clientX:event.clientX,clientY:event.clientY};});</script><button id='target' onclick='window.__clicked=true' style='width:80px;height:40px'>go</button><div id='wheel' style='width:200px;height:200px'>wheel-target</div>";
    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": page,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));
    let captured = send_bidi_command_response(
        &mut socket,
        90,
        "browsingContext.captureScreenshot",
        json!({"context": context_id.clone()}),
    )
    .await;
    assert_eq!(captured["type"], "success");

    let button = send_bidi_command_response(
        &mut socket,
        4,
        "script.evaluate",
        json!({
            "expression": "document.getElementById('target')",
            "target": {
                "context": context_id.clone()
            },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(button["type"], json!("success"));
    let button_shared_id = button["result"]["result"]["sharedId"]
        .as_str()
        .expect("button remote value should include sharedId")
        .to_owned();

    let wheel_target = send_bidi_command_response(
        &mut socket,
        5,
        "script.evaluate",
        json!({
            "expression": "document.getElementById('wheel')",
            "target": {
                "context": context_id.clone()
            },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(wheel_target["type"], json!("success"));
    let wheel_shared_id = wheel_target["result"]["result"]["sharedId"]
        .as_str()
        .expect("wheel target remote value should include sharedId")
        .to_owned();

    let click = send_bidi_command_response(
        &mut socket,
        6,
        "input.performActions",
        json!({
            "context": context_id.clone(),
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    {
                        "type": "pointerMove",
                        "origin": { "type": "element", "sharedId": button_shared_id },
                        "x": 0,
                        "y": 0
                    },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(click["type"], json!("success"), "click response: {click:?}");

    let clicked = send_bidi_command_response(
        &mut socket,
        7,
        "script.evaluate",
        json!({
            "expression": "Boolean(window.__clicked)",
            "target": {
                "context": context_id.clone()
            },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(clicked["type"], json!("success"));
    assert_eq!(clicked["result"]["result"]["value"], json!(true));

    let scroll = send_bidi_command_response(
        &mut socket,
        8,
        "input.performActions",
        json!({
            "context": context_id.clone(),
            "actions": [{
                "type": "wheel",
                "id": "wheel",
                "actions": [{
                    "type": "scroll",
                    "origin": { "type": "element", "sharedId": wheel_shared_id },
                    "x": 1,
                    "y": 2,
                    "deltaX": 7,
                    "deltaY": 13
                }]
            }]
        }),
    )
    .await;
    assert_eq!(
        scroll["type"],
        json!("success"),
        "scroll response: {scroll:?}"
    );

    let wheel = send_bidi_command_response(
        &mut socket,
        9,
        "script.evaluate",
        json!({
            "expression": "window.__wheel ? [window.__wheel.type, window.__wheel.deltaX, window.__wheel.deltaY].join(':') : 'null'",
            "target": {
                "context": context_id.clone()
            },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(wheel["type"], json!("success"));
    assert_eq!(
        wheel["result"]["result"]["value"],
        json!("wheel:7:13"),
        "element-origin wheel should dispatch at real geometry: {wheel:?}"
    );

    let missing_origin = send_bidi_command_response(
        &mut socket,
        10,
        "input.performActions",
        json!({
            "context": context_id.clone(),
            "actions": [{
                "type": "pointer",
                "id": "missing-origin-mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [{
                    "type": "pointerMove",
                    "origin": { "type": "element", "sharedId": "missing-shared-id" },
                    "x": 0,
                    "y": 0
                }]
            }]
        }),
    )
    .await;
    assert_bidi_error(
        &missing_origin,
        "no such node",
        "input.performActions should reject missing element-origin sharedId",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
