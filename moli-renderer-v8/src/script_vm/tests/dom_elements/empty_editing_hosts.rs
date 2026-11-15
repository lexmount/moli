use super::*;

#[test]
fn empty_editing_hosts_retain_a_real_line_without_creating_a_selection() {
    for (html, style, height) in [
        ("<div contenteditable id=t></div>", "", 32.0),
        ("<div contenteditable=plaintext-only id=t></div>", "", 32.0),
        ("<div contenteditable id=t> &#10;&#9; </div>", "", 32.0),
        ("<div contenteditable id=t><!--empty--></div>", "", 32.0),
        ("<div contenteditable id=t><span></span></div>", "", 32.0),
        (
            "<div contenteditable id=t><span style='display:none'>hidden</span></div>",
            "",
            32.0,
        ),
        (
            "<div contenteditable id=t></div>",
            "display:flow-root",
            32.0,
        ),
        (
            "<div contenteditable id=t></div>",
            "display:inline-block",
            32.0,
        ),
        ("<div contenteditable id=t></div>", "display:flex", 32.0),
        ("<div contenteditable id=t></div>", "display:grid", 32.0),
        (
            "<div contenteditable id=t></div>",
            "display:grid;grid-template-rows:70px",
            70.0,
        ),
        ("<div contenteditable id=t></div>", "height:0", 0.0),
        ("<div contenteditable id=t></div>", "max-height:10px", 10.0),
        ("<div id=t></div>", "", 0.0),
        ("<div contenteditable=false id=t></div>", "", 0.0),
        ("<div contenteditable=\" true \" id=t></div>", "", 0.0),
        ("<div contenteditable><div id=t></div></div>", "", 0.0),
        (
            "<div contenteditable><div contenteditable=true id=t></div></div>",
            "",
            0.0,
        ),
        (
            "<div contenteditable><div contenteditable=false id=t></div></div>",
            "",
            0.0,
        ),
        (
            "<div contenteditable=false><div contenteditable id=t></div></div>",
            "",
            32.0,
        ),
        ("<div contenteditable inert id=t></div>", "", 0.0),
        ("<div inert><div contenteditable id=t></div></div>", "", 0.0),
        (
            "<table><tr><td contenteditable id=t></td></tr></table>",
            "padding:0",
            32.0,
        ),
    ] {
        let markup = format!(
            "<!doctype html><style>body{{margin:0;font:16px/32px serif}}#t{{{style}}}</style><body>{html}</body>"
        );
        let mut vm = new_parsed_test_vm("https://empty-editing-host.test/", &markup);
        let result=vm.eval("JSON.stringify([document.getElementById('t').getBoundingClientRect().height,getSelection().rangeCount])").unwrap();
        let actual: [f64; 2] = serde_json::from_str(&result).unwrap();
        assert!(
            (actual[0] - height).abs() < 0.02,
            "{html}/{style}: {result}"
        );
        assert_eq!(actual[1], 0.0, "layout must not fabricate a Selection");
    }
}

#[test]
fn empty_editing_line_uses_normal_font_metrics_and_the_physical_block_axis() {
    let mut vm = new_parsed_test_vm(
        "https://empty-editing-metrics.test/",
        "<!doctype html><div id=reference>x</div><div id=t contenteditable></div>",
    );
    let result=vm.eval("JSON.stringify([reference.getBoundingClientRect().height,t.getBoundingClientRect().height])").unwrap();
    let actual: [f64; 2] = serde_json::from_str(&result).unwrap();
    assert!(actual[0] > 0.0);
    assert!((actual[0] - actual[1]).abs() < 0.02, "{result}");
    for (style, width, height) in [
        ("", 32.0, 0.0),
        ("width:0", 0.0, 0.0),
        ("max-width:10px", 10.0, 0.0),
        ("min-width:50px", 50.0, 0.0),
        ("height:70px", 32.0, 70.0),
    ] {
        let markup = format!(
            "<!doctype html><style>#t{{writing-mode:vertical-rl;font:16px/32px serif;{style}}}</style><div id=t contenteditable></div>"
        );
        let mut vm = new_parsed_test_vm("https://empty-editing-vertical.test/", &markup);
        let result = vm.eval("JSON.stringify([t.getBoundingClientRect().width,t.getBoundingClientRect().height])").unwrap();
        let actual: [f64; 2] = serde_json::from_str(&result).unwrap();
        assert!(
            (actual[0] - width).abs() < 0.02 && (actual[1] - height).abs() < 0.02,
            "{style}: {result}"
        );
    }
}

#[test]
fn empty_body_editing_host_retains_a_line_in_standards_and_quirks_modes() {
    for doctype in ["<!doctype html>", ""] {
        let markup = format!(
            "{doctype}<style>body{{font:16px/32px serif}}</style><body contenteditable></body>"
        );
        let mut vm = new_parsed_test_vm("https://empty-body-editor.test/", &markup);
        assert_eq!(
            vm.eval("String(document.body.getBoundingClientRect().height)")
                .unwrap(),
            "32"
        );
    }
}

#[test]
fn empty_editing_struts_do_not_add_scrollable_ink() {
    for (style, height) in [("", 32), ("height:0", 0), ("max-height:10px", 10)] {
        let markup = format!(
            "<!doctype html><div id=t contenteditable style='font:16px/32px serif;{style}'></div>"
        );
        let mut vm = new_parsed_test_vm("https://empty-editing-scroll.test/", &markup);
        assert_eq!(
            vm.eval("String(t.scrollHeight)").unwrap(),
            height.to_string(),
            "{style}"
        );
    }
    for mode in ["vertical-rl", "vertical-lr"] {
        let markup = format!(
            "<!doctype html><div id=t contenteditable style='writing-mode:{mode};font:16px/32px serif'><span></span></div>"
        );
        let mut vm = new_parsed_test_vm("https://empty-vertical-scroll.test/", &markup);
        assert_eq!(
            vm.eval("JSON.stringify([t.scrollWidth,t.scrollHeight])")
                .unwrap(),
            "[32,0]",
            "{mode}"
        );
    }
}

#[test]
fn editing_host_struts_keep_real_atomic_content_overflow() {
    let mut vm = new_parsed_test_vm(
        "https://editing-atomic-scroll.test/",
        "<!doctype html><div id=t contenteditable style='font:16px/32px serif;height:0'><span style='display:inline-block;vertical-align:top;width:10px;height:40px'></span></div>",
    );
    assert!(vm.eval("t.scrollHeight >= 40").unwrap() == "true");
}
