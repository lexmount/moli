use super::*;

#[test]
fn actions_decode_physical_key_code_and_location_from_raw_webdriver_values() {
    let context = ClassicDevToolsCommandContext::with_target_id("key-locations", "TID-1");
    for (raw, key, code, location, modifier) in [
        ('\u{E008}', "Shift", "ShiftLeft", 1, 8),
        ('\u{E009}', "Control", "ControlLeft", 1, 2),
        ('\u{E00A}', "Alt", "AltLeft", 1, 1),
        ('\u{E03D}', "Meta", "MetaLeft", 1, 4),
        ('\u{E050}', "Shift", "ShiftRight", 2, 8),
        ('\u{E051}', "Control", "ControlRight", 2, 2),
        ('\u{E052}', "Alt", "AltRight", 2, 1),
        ('\u{E053}', "Meta", "MetaRight", 2, 4),
        ('\u{E006}', "Enter", "Enter", 0, 0),
        ('\u{E007}', "Enter", "NumpadEnter", 1, 0),
        ('\u{E019}', "=", "NumpadEqual", 3, 0),
        ('\u{E01A}', "0", "Numpad0", 3, 0),
        ('\u{E023}', "9", "Numpad9", 3, 0),
        ('\u{E024}', "*", "NumpadMultiply", 3, 0),
        ('\u{E025}', "+", "NumpadAdd", 3, 0),
        ('\u{E026}', ",", "NumpadComma", 3, 0),
        ('\u{E027}', "-", "NumpadSubtract", 3, 0),
        ('\u{E028}', ".", "NumpadDecimal", 3, 0),
        ('\u{E029}', "/", "NumpadDivide", 3, 0),
        ('\u{E054}', "PageUp", "Numpad9", 3, 0),
        ('\u{E055}', "PageDown", "Numpad3", 3, 0),
        ('\u{E056}', "End", "Numpad1", 3, 0),
        ('\u{E057}', "Home", "Numpad7", 3, 0),
        ('\u{E058}', "ArrowLeft", "Numpad4", 3, 0),
        ('\u{E059}', "ArrowUp", "Numpad8", 3, 0),
        ('\u{E05A}', "ArrowRight", "Numpad6", 3, 0),
        ('\u{E05B}', "ArrowDown", "Numpad2", 3, 0),
        ('\u{E05C}', "Insert", "Numpad0", 3, 0),
        ('\u{E05D}', "Delete", "NumpadDecimal", 3, 0),
        ('\u{E012}', "ArrowLeft", "ArrowLeft", 0, 0),
        ('a', "a", "", 0, 0),
        ('=', "=", "", 0, 0),
    ] {
        let commands = perform_actions_commands(
            &context,
            &json!({"actions":[{
                "type":"key","id":"keyboard","actions":[
                    {"type":"keyDown","value":raw.to_string()},
                    {"type":"keyUp","value":raw.to_string()}
                ]
            }]}),
        )
        .unwrap();
        assert_eq!(commands.len(), 2, "{raw:?}");
        for (command, event_type) in commands
            .into_iter()
            .zip([DevToolsKeyEventType::KeyDown, DevToolsKeyEventType::KeyUp])
        {
            let AutomationCommand::DispatchKeyEvent(command) = command else {
                panic!("expected key dispatch")
            };
            assert_eq!(
                (
                    command.event_type,
                    command.key.as_str(),
                    command.code.as_str(),
                    command.location
                ),
                (event_type, key, code, location),
                "{raw:?}"
            );
            assert_eq!(
                command.modifiers,
                if event_type == DevToolsKeyEventType::KeyDown {
                    modifier
                } else {
                    0
                },
                "{raw:?}"
            );
            assert!(!command.auto_repeat, "{raw:?}");
        }
    }
}

#[test]
fn release_actions_retains_decoded_key_locations_and_repeats() {
    let context = ClassicDevToolsCommandContext::with_target_id("key-locations", "TID-1");
    let mut state = ClassicActionState::default();
    let commands = perform_actions_commands_with_state(
        &context,
        &json!({"actions":[{
            "type":"key","id":"keyboard","actions":[
                {"type":"keyDown","value":"\u{E050}"},
                {"type":"keyDown","value":"\u{E050}"},
                {"type":"keyDown","value":"\u{E019}"}
            ]
        }]}),
        &ClassicElementOriginViewportPoints::default(),
        &mut state,
    )
    .unwrap();
    assert_eq!(commands.len(), 3);
    for (command, code, location, repeat) in [
        (&commands[0], "ShiftRight", 2, false),
        (&commands[1], "ShiftRight", 2, true),
        (&commands[2], "NumpadEqual", 3, false),
    ] {
        let AutomationCommand::DispatchKeyEvent(command) = command else {
            panic!("expected key dispatch")
        };
        assert_eq!(
            (command.code.as_str(), command.location, command.auto_repeat),
            (code, location, repeat)
        );
    }
    let released = release_actions_commands(&context, &mut state);
    assert_eq!(released.len(), 2);
    for (command, code, location) in [
        (&released[0], "NumpadEqual", 3),
        (&released[1], "ShiftRight", 2),
    ] {
        let AutomationCommand::DispatchKeyEvent(command) = command else {
            panic!("expected key dispatch")
        };
        assert_eq!(
            (command.event_type, command.code.as_str(), command.location),
            (DevToolsKeyEventType::KeyUp, code, location)
        );
    }
    assert!(release_actions_commands(&context, &mut state).is_empty());
}

#[test]
fn element_send_keys_preserves_right_modifier_and_numpad_locations() {
    let context = ClassicDevToolsCommandContext::with_target_id("key-locations", "TID-1");
    let commands = element_send_keys_input_commands(&context, "\u{E050}\u{E000}\u{E019}");
    assert_eq!(commands.len(), 4);
    for (command, code, location) in commands
        .iter()
        .zip([
            ("ShiftRight", 2),
            ("ShiftRight", 2),
            ("NumpadEqual", 3),
            ("NumpadEqual", 3),
        ])
        .map(|(c, (code, loc))| (c, code, loc))
    {
        let AutomationCommand::DispatchKeyEvent(command) = command else {
            panic!("expected key dispatch")
        };
        assert_eq!((command.code.as_str(), command.location), (code, location));
    }
}
