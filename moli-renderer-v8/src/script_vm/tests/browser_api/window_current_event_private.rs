use super::*;

#[test]
fn window_current_events_ignore_author_slot_names_and_public_replacements() {
    let mut vm = new_storage_html_test_vm("https://private-window-event.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'; 'ready'")
        .unwrap();
    vm.eval(include_str!("window_current_event_private.js"))
        .unwrap();
    let facts: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(__uiEventResults)").unwrap()).unwrap();
    assert_eq!(facts["rows"].as_array().unwrap().len(), 12);
    assert_eq!(facts["total"], 378);
    assert_eq!(facts["complete"], true, "{facts}");
}

#[test]
fn window_current_events_use_private_state_in_each_listener_world() {
    let mut vm = new_storage_html_test_vm("https://private-window-event-worlds.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<div id=target></div>';
        globalThis.target = document.getElementById('target');
        globalThis.facts = [];
        Object.defineProperty(globalThis, '__moliWindowEvent', {
            get() { throw new Error('author slot getter'); },
            set() { throw new Error('author slot setter'); }
        });
        target.addEventListener('outer', event => {
            facts.push(window.event === event);
            target.dispatchEvent(new Event('inner'));
            facts.push(window.event === event);
        });
        'ready'
        "#,
    )
    .unwrap();
    let isolated = vm
        .create_isolated_world("private-current-event", false)
        .unwrap();
    vm.eval_in_isolated_context(
        isolated,
        r#"
        globalThis.target = document.getElementById('target');
        globalThis.facts = [];
        Object.defineProperty(globalThis, '__moliWindowEvent', {
            get() { throw new Error('isolated author slot getter'); },
            set() { throw new Error('isolated author slot setter'); }
        });
        target.addEventListener('inner', event => {
            facts.push(window.event === event);
        });
        'ready'
        "#,
    )
    .unwrap();
    assert_eq!(
        vm.eval("target.dispatchEvent(new Event('outer')); JSON.stringify([facts, window.event === undefined])")
            .unwrap(),
        "[[true,true],true]"
    );
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            "JSON.stringify([facts, window.event === undefined])"
        )
        .unwrap(),
        "[[true],true]"
    );
}
