use super::*;

#[test]
fn empty_editing_hosts_retain_one_line_before_focus() {
    for (html, expected) in [
        ("<div id=t contenteditable></div>", 24.0),
        ("<div id=t contenteditable=plaintext-only></div>", 24.0),
        ("<div id=t contenteditable> \n \t </div>", 24.0),
        ("<div id=t contenteditable><span></span></div>", 24.0),
        (
            "<span id=t contenteditable style='display:inline-block'></span>",
            24.0,
        ),
        (
            "<div id=t contenteditable style='display:flow-root'></div>",
            24.0,
        ),
        (
            "<div id=t contenteditable style='display:flex'></div>",
            24.0,
        ),
        (
            "<div id=t contenteditable style='display:grid'></div>",
            24.0,
        ),
        (
            "<div id=t contenteditable style='display:grid;grid-template-rows:80px'></div>",
            80.0,
        ),
        (
            "<div id=t contenteditable style='display:table-cell'></div>",
            24.0,
        ),
        (
            "<div id=t contenteditable style='display:list-item;list-style:none'></div>",
            24.0,
        ),
        (
            "<div id=t contenteditable style='padding:3px;border:2px solid'></div>",
            34.0,
        ),
        ("<div id=t contenteditable style='height:8px'></div>", 8.0),
        (
            "<div id=t contenteditable style='max-height:8px'></div>",
            8.0,
        ),
        ("<div id=t contenteditable style='height:0'></div>", 0.0),
        ("<div id=t></div>", 0.0),
        ("<div id=t contenteditable=false></div>", 0.0),
        ("<div contenteditable><div id=t></div></div>", 0.0),
        (
            "<div contenteditable><div id=t contenteditable=true></div></div>",
            0.0,
        ),
        (
            "<div id=t contenteditable><div contenteditable=false></div></div>",
            0.0,
        ),
        (
            "<div id=t contenteditable style='display:contents'></div>",
            0.0,
        ),
        ("<div id=t contenteditable style='display:none'></div>", 0.0),
        (
            "<div id=t contenteditable style='content-visibility:hidden'></div>",
            0.0,
        ),
    ] {
        let mut vm = new_parsed_test_vm(
            "https://empty-editor.test/",
            &format!("<!doctype html><style>body{{margin:0;font:16px/24px Arial}}</style>{html}"),
        );
        let query = "document.getElementById('t').getBoundingClientRect().height";
        let height = vm.eval(query).unwrap().parse::<f64>().unwrap();
        assert!(
            (height - expected).abs() < 0.02,
            "{html}: height {height}, expected {expected}"
        );
        assert_eq!(
            vm.eval("document.activeElement === document.getElementById('t')")
                .unwrap(),
            "false"
        );
        publish_layout_for_test(&mut vm);
        assert_eq!(
            vm.eval(query).unwrap().parse::<f64>().unwrap(),
            height,
            "{html}: retained layout"
        );
    }
}

#[test]
fn empty_editing_host_line_uses_font_metrics_and_preserves_orthogonal_geometry() {
    for style in [
        "font:16px Arial",
        "font:24px monospace",
        "font:16px/32px Arial",
    ] {
        let mut vm = new_parsed_test_vm(
            "https://editor-font-strut.test/",
            &format!(
                "<!doctype html><style>body{{margin:0;{style}}}</style><div id=editor contenteditable></div><div id=reference>A</div>"
            ),
        );
        assert_eq!(vm.eval("editor.getBoundingClientRect().height === reference.getBoundingClientRect().height && editor.getBoundingClientRect().height > 0").unwrap(),"true","{style}");
    }
    // Plain empty inline boxes already expose the backend's font box. Keep
    // an inline editing host identical to that control rather than giving it
    // the new block editing-host line.
    let mut vm = new_parsed_test_vm(
        "https://inline-editor.test/",
        "<!doctype html><style>body{font:16px/24px Arial}</style><div><span id=editor contenteditable></span></div><div><span id=reference></span></div>",
    );
    assert_eq!(vm.eval("JSON.stringify([editor.getBoundingClientRect().height === reference.getBoundingClientRect().height,editor.getBoundingClientRect().width === reference.getBoundingClientRect().width])").unwrap(),"[true,true]");

    // Orthogonal block sizing is a pre-existing backend limitation. The
    // horizontal empty-line behavior must not add a physical height there.
    let mut vm = new_parsed_test_vm(
        "https://vertical-editor.test/",
        "<!doctype html><style>.v{font:16px/24px Arial;writing-mode:vertical-rl}</style><div id=editor class=v contenteditable></div><div id=reference class=v></div>",
    );
    assert_eq!(vm.eval("JSON.stringify([editor.getBoundingClientRect().width === reference.getBoundingClientRect().width,editor.getBoundingClientRect().height,reference.getBoundingClientRect().height])").unwrap(), "[true,0,0]");
}

#[test]
fn editing_host_line_tracks_content_and_editable_state_without_a_caret() {
    let mut vm = new_parsed_test_vm(
        "https://editor-mutation.test/",
        "<!doctype html><div id=editor contenteditable style='font:16px/24px Arial'></div>",
    );
    for (change, height) in [
        ("", 24),
        ("editor.removeAttribute('contenteditable')", 0),
        ("editor.setAttribute('contenteditable','true')", 24),
        ("editor.innerHTML='<div></div>'", 0),
        ("editor.textContent=''", 24),
        ("editor.innerHTML='<br><br>'", 48),
        ("editor.textContent=''", 24),
    ] {
        vm.eval(change).unwrap();
        // Synchronous geometry reads consume Moli's published snapshot.
        publish_layout_for_test(&mut vm);
        assert_eq!(
            vm.eval("editor.getBoundingClientRect().height").unwrap(),
            height.to_string(),
            "{change}"
        );
    }
}
