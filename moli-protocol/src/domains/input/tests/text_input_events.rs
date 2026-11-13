use super::*;

const PROBE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../moli-renderer-v8/tests/fixtures/text-input-events.js"
));

async fn fixture(kind: &str, options: serde_json::Value) -> TestContext {
    let mut ctx = TestContext::new();
    with_loaded_document(&mut ctx, "<html><body></body></html>").await;
    assert_eq!(
        evaluate_string(
            &mut ctx,
            &format!(
                "{PROBE}\ninstallTextInputProbe({}, {})",
                serde_json::to_string(kind).unwrap(),
                options,
            )
        )
        .await,
        "ready"
    );
    ctx
}

async fn command(ctx: &mut TestContext, method: &str, params: serde_json::Value) {
    ctx.process_async(json!({"id": 951,"method":method,"params":params}))
        .await;
    ctx.expect_result(951, json!({}), None);
}

async fn type_text(ctx: &mut TestContext, route: &str, text: &str) {
    if route == "insertText" {
        command(ctx, "Input.insertText", json!({"text":text})).await;
        return;
    }
    if route == "raw-char" {
        command(
            ctx,
            "Input.dispatchKeyEvent",
            json!({"type":"rawKeyDown","key":"a","code":"KeyA","text":text}),
        )
        .await;
    }
    command(ctx,"Input.dispatchKeyEvent",json!({"type":if route=="keyDown" {"keyDown"} else {"char"},"key":"a","code":"KeyA","text":text})).await;
    command(
        ctx,
        "Input.dispatchKeyEvent",
        json!({"type":"keyUp","key":"a","code":"KeyA"}),
    )
    .await;
}

async fn read(ctx: &mut TestContext) -> serde_json::Value {
    serde_json::from_str(&evaluate_string(ctx, "JSON.stringify(readTextInputProbe())").await)
        .unwrap()
}

fn text_event(target: &str, original: &str, data: &str, value: &str) -> serde_json::Value {
    json!({"type":"textInput","target":target,"originalTarget":original,"data":data,
        "native":true,"ui":true,"input":false,"trusted":true,"bubbles":true,
        "cancelable":true,"composed":true,"view":true,"detail":0,"value":value})
}

fn sequence(route: &str, target: &str) -> Vec<String> {
    let mut types = Vec::new();
    if matches!(route, "keyDown" | "raw-char") {
        types.push("keydown");
    }
    if route != "insertText" {
        types.push("keypress");
    }
    types.extend(["beforeinput", "textInput", "input"]);
    if route != "insertText" {
        types.push("keyup");
    }
    types
        .into_iter()
        .map(|event| format!("{event}:{target}"))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_text_input_dispatches_native_legacy_events_in_character_routes() {
    for kind in ["input", "textarea", "editor", "shadow", "frame"] {
        for route in ["keyDown", "char", "raw-char", "insertText"] {
            for text in [
                "a",
                if route == "insertText" {
                    "😀ab"
                } else {
                    "😀a"
                },
            ] {
                let mut ctx = fixture(kind, json!({})).await;
                type_text(&mut ctx, route, text).await;
                let observed = read(&mut ctx).await;
                let target = if kind == "shadow" { "host" } else { "field" };
                assert_eq!(
                    observed["events"],
                    json!(sequence(route, target)),
                    "{kind}/{route}/{text}: {observed}"
                );
                assert_eq!(
                    observed["textEvents"],
                    json!([text_event(target, "field", text, "")]),
                    "{kind}/{route}: {observed}"
                );
                assert_eq!(observed["value"], text, "{kind}/{route}: {observed}");
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_text_input_cancellation_preserves_the_value_and_gates_later_events() {
    for kind in ["input", "textarea", "editor"] {
        for route in ["keyDown", "char", "insertText"] {
            for cancel in ["keydown", "keypress", "beforeinput", "textInput"] {
                let full = sequence(route, "field");
                let Some(index) = full
                    .iter()
                    .position(|event| event == &format!("{cancel}:field"))
                else {
                    continue;
                };
                let mut ctx = fixture(kind, json!({"cancel":cancel})).await;
                type_text(&mut ctx, route, "a").await;
                let observed = read(&mut ctx).await;
                let mut expected = full[..=index].to_vec();
                if route != "insertText" {
                    expected.push("keyup:field".into());
                }
                assert_eq!(
                    observed["events"],
                    json!(expected),
                    "{kind}/{route}/{cancel}: {observed}"
                );
                assert_eq!(observed["value"], "", "{kind}/{route}/{cancel}: {observed}");
                assert_eq!(
                    observed["textEvents"].as_array().unwrap().len(),
                    usize::from(cancel == "textInput")
                );
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_text_input_revalidates_targets_after_legacy_listeners() {
    for (kind, effects) in [
        ("input", vec!["remove", "readonly", "adopt"]),
        ("textarea", vec!["remove", "readonly", "adopt"]),
        ("editor", vec!["remove", "disable", "adopt"]),
        ("frame", vec!["remove-frame"]),
    ] {
        for route in ["keyDown", "insertText"] {
            for effect in &effects {
                let mut ctx = fixture(kind, json!({"phase":"textInput","effect":effect})).await;
                type_text(&mut ctx, route, "X").await;
                let observed = read(&mut ctx).await;
                assert_eq!(
                    observed["textEvents"].as_array().unwrap().len(),
                    1,
                    "{kind}/{route}/{effect}: {observed}"
                );
                assert_eq!(observed["value"], "", "{kind}/{route}/{effect}: {observed}");
                assert!(
                    !observed["events"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|event| event.as_str().unwrap().starts_with("input:")),
                    "{observed}"
                );
            }
        }
    }
    for kind in ["input", "textarea"] {
        for route in ["keyDown", "insertText"] {
            let mut ctx = fixture(kind, json!({"phase":"textInput","effect":"selection"})).await;
            type_text(&mut ctx, route, "X").await;
            let observed = read(&mut ctx).await;
            assert_eq!(observed["value"], "aXc", "{kind}/{route}: {observed}");
            let caret = if route == "insertText" { 1 } else { 2 };
            assert_eq!(
                observed["selection"],
                json!([caret, caret]),
                "{kind}/{route}: {observed}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_text_input_resolves_keyboard_and_ime_focus_at_their_own_boundaries() {
    for kind in ["input", "textarea", "editor"] {
        for route in ["keyDown", "insertText"] {
            for phase in ["beforeinput", "textInput"] {
                let mut ctx = fixture(kind, json!({"phase":phase,"effect":"focus"})).await;
                type_text(&mut ctx, route, "X").await;
                let observed = read(&mut ctx).await;
                let target = if route == "insertText" && phase == "beforeinput" {
                    "other"
                } else {
                    "field"
                };
                assert_eq!(
                    observed["textEvents"],
                    json!([text_event(target, target, "X", "")]),
                    "{kind}/{route}/{phase}: {observed}"
                );
                let edits_other = target == "other" || kind == "editor";
                assert_eq!(
                    observed["value"],
                    if edits_other { "" } else { "X" },
                    "{observed}"
                );
                assert_eq!(
                    observed["other"],
                    if edits_other { "X" } else { "" },
                    "{observed}"
                );
                assert_eq!(
                    evaluate_string(
                        &mut ctx,
                        "textInputProbe.field.ownerDocument.activeElement.id"
                    )
                    .await,
                    if edits_other { "other" } else { "field" },
                    "native insertion must restore its selection owner's focus: {observed}",
                );
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_text_input_preserves_filtered_text_and_intrinsic_event_factories() {
    for kind in ["input", "shadow", "frame"] {
        let mut ctx = fixture(kind, json!({"poison":true})).await;
        type_text(&mut ctx, "keyDown", "X").await;
        let observed = read(&mut ctx).await;
        let target = if kind == "shadow" { "host" } else { "field" };
        assert_eq!(
            observed["textEvents"],
            json!([text_event(target, "field", "X", "")]),
            "{observed}"
        );
        assert_eq!(observed["value"], "X");
    }
    for kind in ["input", "textarea"] {
        for route in ["keyDown", "insertText"] {
            let mut ctx = fixture(kind, json!({"maxLength":1})).await;
            type_text(&mut ctx, route, "XYZ").await;
            let observed = read(&mut ctx).await;
            assert_eq!(
                observed["textEvents"],
                json!([text_event("field", "field", "XYZ", "")])
            );
            assert_eq!(observed["value"], "X");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_text_input_skips_raw_keys_deletion_and_script_commands() {
    for event in [
        json!({"type":"rawKeyDown","key":"a","code":"KeyA","text":"a"}),
        json!({"type":"keyDown","key":"Backspace","code":"Backspace"}),
        json!({"type":"keyDown","key":"ArrowLeft","code":"ArrowLeft"}),
    ] {
        let mut ctx = fixture("input", json!({})).await;
        command(&mut ctx, "Input.dispatchKeyEvent", event).await;
        assert_eq!(read(&mut ctx).await["textEvents"], json!([]));
    }
    let mut ctx = fixture("input", json!({})).await;
    evaluate_bool(&mut ctx, "document.execCommand('insertText',false,'a')").await;
    let observed = read(&mut ctx).await;
    assert_eq!(observed["textEvents"], json!([]));
    assert_eq!(observed["value"], "a");
    for route in ["keyDown", "insertText"] {
        let mut ctx = fixture("readonly", json!({})).await;
        type_text(&mut ctx, route, "a").await;
        let observed = read(&mut ctx).await;
        assert_eq!(
            observed["textEvents"],
            json!([text_event("field", "field", "a", "")])
        );
        assert_eq!(observed["value"], "");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_text_input_line_breaks_and_non_editable_targets_match_chromium() {
    for kind in ["input", "textarea", "editor"] {
        let mut ctx = fixture(kind, json!({})).await;
        command(&mut ctx,"Input.dispatchKeyEvent",json!({"type":"keyDown","key":"Enter","code":"Enter","text":"\r","windowsVirtualKeyCode":13})).await;
        let observed = read(&mut ctx).await;
        if kind == "input" {
            assert_eq!(observed["textEvents"], json!([]));
        } else {
            assert_eq!(
                observed["textEvents"],
                json!([text_event("field", "field", "\n", "")]),
                "{kind}: {observed}"
            );
            let input_type = if kind == "textarea" {
                "insertLineBreak"
            } else {
                "insertParagraph"
            };
            assert_eq!(
                observed["editEvents"],
                json!([
                    {"type":"beforeinput","inputType":input_type,"data":null},
                    {"type":"input","inputType":input_type,"data":null},
                ]),
                "{observed}"
            );
        }
    }
    for route in ["keyDown", "insertText"] {
        let mut ctx = fixture("plain", json!({})).await;
        type_text(&mut ctx, route, "a").await;
        let observed = read(&mut ctx).await;
        assert_eq!(observed["textEvents"], json!([]), "{observed}");
        assert_eq!(observed["value"], "");
        if route == "insertText" {
            assert_eq!(observed["events"], json!([]), "{observed}");
        }
    }
    let mut ctx = fixture("input", json!({})).await;
    command(&mut ctx, "Input.insertText", json!({"text":""})).await;
    assert_eq!(read(&mut ctx).await["events"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_insert_text_replaces_selection_and_remains_cancelable() {
    for kind in ["input", "textarea", "div"] {
        for canceled in [false, true] {
            let mut ctx = TestContext::new();
            let html = match kind {
                "input" => "<input id=editor value=abc>",
                "textarea" => "<textarea id=editor>abc</textarea>",
                _ => "<div id=editor contenteditable=true>abc</div>",
            };
            with_loaded_document(&mut ctx, html).await;
            let setup = format!(
                r#"(() => {{
                const editor=document.getElementById('editor'); editor.focus();
                if ('{kind}'==='div') {{const r=document.createRange();r.setStart(editor.firstChild,1);r.setEnd(editor.firstChild,2);getSelection().removeAllRanges();getSelection().addRange(r);}}
                else editor.setSelectionRange(1,2);
                window.emptyEvents=[];
                for(const t of ['beforeinput','textInput','input']) editor.addEventListener(t,e=>{{emptyEvents.push([e.type,e.data]);if({canceled}&&t==='beforeinput')e.preventDefault();}});
                return 'ready';
            }})()"#
            );
            assert_eq!(evaluate_string(&mut ctx, &setup).await, "ready");
            command(&mut ctx, "Input.insertText", json!({"text":""})).await;
            let result: serde_json::Value = serde_json::from_str(&evaluate_string(&mut ctx,
                "JSON.stringify([document.getElementById('editor').value??document.getElementById('editor').textContent,emptyEvents])").await).unwrap();
            assert_eq!(
                result,
                if canceled {
                    json!(["abc", [["beforeinput", ""]]])
                } else {
                    json!([
                        "ac",
                        [["beforeinput", ""], ["textInput", ""], ["input", ""]]
                    ])
                },
                "{kind}, canceled={canceled}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ime_retains_caret_captured_before_beforeinput() {
    for tag in ["input", "textarea"] {
        for move_focus in [false, true] {
            let mut ctx = TestContext::new();
            let html = if tag == "input" {
                "<input id=first value=abcd><input id=second value=wxyz>"
            } else {
                "<textarea id=first>abcd</textarea><textarea id=second>wxyz</textarea>"
            };
            with_loaded_document(&mut ctx, html).await;
            let setup = format!(
                r#"(() => {{const first=document.getElementById('first'),second=document.getElementById('second');first.focus();first.setSelectionRange(1,2);second.setSelectionRange(2,3);first.addEventListener('beforeinput',()=>{{if({move_focus})second.focus();else first.setSelectionRange(3,4);}});return 'ready';}})()"#
            );
            assert_eq!(evaluate_string(&mut ctx, &setup).await, "ready");
            command(&mut ctx, "Input.insertText", json!({"text":"X"})).await;
            let observed:serde_json::Value=serde_json::from_str(&evaluate_string(&mut ctx,"JSON.stringify(['first','second'].map(id=>{const e=document.getElementById(id);return [e.value,e.selectionStart,e.selectionEnd]}))").await).unwrap();
            assert_eq!(
                observed,
                if move_focus {
                    json!([["abcd", 1, 2], ["wxXz", 2, 2]])
                } else {
                    json!([["abcX", 2, 2], ["wxyz", 2, 3]])
                },
                "tag={tag}, move_focus={move_focus}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn iframe_contenteditable_ime_revalidates_selection_focus_and_owner_after_callbacks() {
    for phase in ["beforeinput", "textInput"] {
        for effect in ["selection", "focus", "remove", "adopt"] {
            let mut ctx = TestContext::new();
            with_loaded_document(&mut ctx, "<html><body></body></html>").await;
            assert_eq!(evaluate_string(&mut ctx,&format!(r#"(() => {{
                const frame=document.createElement('iframe');document.body.append(frame);
                const owner=frame.contentDocument;
                owner.body.innerHTML='<div id=first contenteditable>abcd</div><div id=second contenteditable>wxyz</div>';
                const first=owner.getElementById('first'),second=owner.getElementById('second');
                const s=owner.defaultView.getSelection();first.focus();
                s.setBaseAndExtent(first.firstChild,1,first.firstChild,2);
                globalThis.editProbe={{owner,first,second,events:[]}};
                for(const node of [first,second])for(const type of ['beforeinput','textInput','input'])
                    node.addEventListener(type,e=>editProbe.events.push([type,e.target.id,e.data]));
                first.addEventListener({phase:?},()=>{{
                    if({effect:?}==='selection')s.setBaseAndExtent(first.firstChild,3,first.firstChild,4);
                    if({effect:?}==='focus')second.focus();
                    if({effect:?}==='remove')first.remove();
                    if({effect:?}==='adopt')document.body.append(document.adoptNode(first));
                }},{{once:true}});return 'ready';
            }})()"#)).await,"ready");
            type_text(&mut ctx, "insertText", "X").await;
            let actual: serde_json::Value=serde_json::from_str(&evaluate_string(&mut ctx,r#"JSON.stringify({first:editProbe.first.textContent,second:editProbe.second.textContent,events:editProbe.events,childOwner:editProbe.first.ownerDocument===editProbe.owner})"#).await).unwrap();
            let mut events = vec![json!(["beforeinput", "first", "X"])];
            if phase == "textInput" || !matches!(effect, "remove" | "adopt") {
                events.push(json!([
                    "textInput",
                    if effect == "focus" && phase == "beforeinput" {
                        "second"
                    } else {
                        "first"
                    },
                    "X"
                ]));
            }
            if !matches!(effect, "remove" | "adopt") {
                events.push(json!([
                    "input",
                    if effect == "focus" { "second" } else { "first" },
                    "X"
                ]));
            }
            assert_eq!(
                actual,
                json!({
                    "first":if effect=="selection" {"abcX"} else {"abcd"},
                    "second":if effect=="focus" {"Xwxyz"} else {"wxyz"},
                    "events":events,"childOwner":effect!="adopt",
                }),
                "{phase}/{effect}"
            );
        }
    }
}
