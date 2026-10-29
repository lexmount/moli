use super::automatic_controls::font_usage_vm;
use super::selection_order::order_font_data;
use super::*;

fn load_text_subset(runs: &[&str], whitespace: &str, range: &str) -> String {
    let mut vm = font_usage_vm();
    let (_, wide) = order_font_data();
    let runs = serde_json::to_string(runs).unwrap();
    vm.eval(&format!(r#"
        globalThis.face=new FontFace('CaseFace','url(data:font/ttf;base64,{wide})',{{unicodeRange:'{range}'}});
        document.fonts.add(face);
        const text=document.createElement('span');
        text.style.cssText='font-family:CaseFace;text-transform:capitalize;white-space:{whitespace}';
        text.append(...{runs}.map(value=>document.createTextNode(value)));
        const container=document.createElement('div');container.append(text);document.body.append(container);
    "#)).unwrap();
    vm.reconcile_document_font_resources();
    vm.eval("document.fonts.ready.then(()=>globalThis.ready=true)")
        .unwrap();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("String(ready)").unwrap(), "true");
    assert_eq!(vm.layout_pass_observability_for_test().1, 0);
    vm.eval("face.status").unwrap()
}

#[test]
fn automatic_fonts_capitalize_after_segment_breaks_and_tabs() {
    for runs in [
        vec!["a\nb"],
        vec!["a\tb"],
        vec!["a\rb"],
        vec!["a\r\nb"],
        vec!["a\r", "\nb"],
    ] {
        assert_eq!(
            load_text_subset(&runs, "normal", "U+0042"),
            "loaded",
            "{runs:?}"
        );
    }
}

#[test]
fn automatic_fonts_preserve_word_state_across_text_nodes_and_combining_marks() {
    for runs in [vec!["ab"], vec!["a", "b"], vec!["a\u{0301}", "b"]] {
        assert_eq!(
            load_text_subset(&runs, "normal", "U+0042"),
            "unloaded",
            "{runs:?}"
        );
        assert_eq!(
            load_text_subset(&runs, "normal", "U+0062"),
            "loaded",
            "{runs:?}"
        );
    }
}

#[test]
fn automatic_fonts_collect_only_surviving_normalized_spaces() {
    for (runs, whitespace, expected) in [
        (vec!["a\nb"], "normal", "loaded"),
        (vec!["a\tb"], "normal", "loaded"),
        (vec!["a\r", "\nb"], "normal", "loaded"),
        (vec!["a\nb"], "pre-line", "unloaded"),
        (vec!["a\nb"], "pre", "unloaded"),
        (vec!["a b"], "pre", "loaded"),
        (vec![" a "], "normal", "unloaded"),
        (vec![" a "], "pre", "loaded"),
        (vec!["a", " ", "b"], "normal", "loaded"),
    ] {
        assert_eq!(
            load_text_subset(&runs, whitespace, "U+0020"),
            expected,
            "{runs:?}/{whitespace}"
        );
    }
}

#[test]
fn automatic_fonts_follow_inline_and_generated_text_order_and_block_boundaries() {
    let (_, wide) = order_font_data();
    for (markup, css, expected) in [
        (
            "<span>a<span style='font-family:CaseFace'>b</span></span>",
            "",
            "unloaded",
        ),
        (
            "<span style='font-family:CaseFace'>b</span>",
            "span::before {content:'a'}",
            "unloaded",
        ),
        (
            "<span style='font-family:CaseFace'>a</span>",
            "span::after {content:'b'}",
            "unloaded",
        ),
        (
            "<span style='font-family:CaseFace'>b</span>",
            "span::before {content:'a '}",
            "loaded",
        ),
        (
            "<span>a</span><div></div><span style='font-family:CaseFace'>b</span>",
            "",
            "loaded",
        ),
        (
            "<span>a<br><span style='font-family:CaseFace'>b</span></span>",
            "",
            "loaded",
        ),
        (
            "<span>a<i style='display:inline-block'>x</i><span style='font-family:CaseFace'>b</span></span>",
            "",
            "unloaded",
        ),
    ] {
        let mut vm = font_usage_vm();
        let markup = serde_json::to_string(markup).unwrap();
        let css = serde_json::to_string(css).unwrap();
        vm.eval(&format!(r#"
            globalThis.face=new FontFace('CaseFace','url(data:font/ttf;base64,{wide})',{{unicodeRange:'U+0042'}});
            document.fonts.add(face);
            const style=document.createElement('style');style.textContent={css};document.head.append(style);
            const container=document.createElement('div');container.style.textTransform='capitalize';
            container.innerHTML={markup};document.body.append(container);
        "#)).unwrap();
        vm.reconcile_document_font_resources();
        run_font_tasks(&mut vm);
        assert_eq!(vm.eval("face.status").unwrap(), expected, "{markup}/{css}");
        assert_eq!(vm.layout_pass_observability_for_test().1, 0);
    }
}

fn assert_contents_subset(markup: &str, range: &str, expected: &str) {
    let mut vm = font_usage_vm();
    let (_, wide) = order_font_data();
    let markup = serde_json::to_string(markup).unwrap();
    vm.eval(&format!(r#"
        globalThis.face=new FontFace('ContentsFace','url(data:font/ttf;base64,{wide})',{{unicodeRange:'{range}'}});
        document.fonts.add(face);
        const style=document.createElement('style');
        style.textContent='[data-generated]::before {{content:"A"}} input,textarea,select,button {{font-family:ContentsFace}}';
        document.head.append(style);
        const container=document.createElement('span');
        container.style.cssText='font-family:ContentsFace;text-transform:capitalize';
        container.innerHTML={markup};document.body.append(container);
    "#)).unwrap();
    vm.reconcile_document_font_resources();
    vm.eval("document.fonts.ready.then(()=>globalThis.ready=true)")
        .unwrap();
    run_font_tasks(&mut vm);
    assert_eq!(
        vm.eval("face.status").unwrap(),
        expected,
        "{markup}/{range}"
    );
    assert_eq!(vm.eval("globalThis.ready===true").unwrap(), "true");
    assert_eq!(vm.layout_pass_observability_for_test().1, 0);
}

#[test]
fn automatic_fonts_contents_suppression_preserves_text_context() {
    for (markup, range, expected) in [
        (
            "<object style='display:contents'>x</object>a",
            "U+0041",
            "loaded",
        ),
        ("<object>x</object>a", "U+0041", "unloaded"),
        (
            "<span style='display:contents'>x</span>a",
            "U+0041",
            "unloaded",
        ),
        (
            "a<object style='display:contents'> </object>b",
            "U+0042",
            "unloaded",
        ),
        (
            "a<object style='display:contents'>\n</object>b",
            "U+0042",
            "unloaded",
        ),
        ("a<br style='display:contents'>b", "U+0042", "unloaded"),
        (
            "<object style='display:contents'><input value='x'></object>a",
            "U+0041",
            "loaded",
        ),
        (
            "x<object style='display:contents'><br></object>b",
            "U+0042",
            "unloaded",
        ),
    ] {
        assert_contents_subset(markup, range, expected);
    }
}

#[test]
fn automatic_fonts_contents_suppression_precedes_control_and_pseudo_text() {
    for (markup, range, expected) in [
        (
            "<object style='display:contents'>AAAA</object>",
            "U+0041",
            "unloaded",
        ),
        (
            "<input style='display:contents' value='AAAA'>",
            "U+0041",
            "unloaded",
        ),
        (
            "<input style='display:contents' placeholder='AAAA'>",
            "U+0041",
            "unloaded",
        ),
        (
            "<textarea style='display:contents'>AAAA</textarea>",
            "U+0041",
            "unloaded",
        ),
        (
            "<select style='display:contents'><option label='AAAA'></option></select>",
            "U+0041",
            "unloaded",
        ),
        (
            "<object data-generated style='display:contents'></object>",
            "U+0041",
            "unloaded",
        ),
        (
            "<wbr data-generated style='display:contents'>",
            "U+0041",
            "unloaded",
        ),
        (
            "<input type='submit' style='display:contents'>",
            "U+0053",
            "unloaded",
        ),
        ("<input value='AAAA'>", "U+0041", "loaded"),
        ("<textarea>AAAA</textarea>", "U+0041", "loaded"),
        (
            "<select><option label='AAAA'></option></select>",
            "U+0041",
            "loaded",
        ),
        ("<input type='submit'>", "U+0053", "loaded"),
        ("<object>AAAA</object>", "U+0041", "loaded"),
        (
            "<span style='display:contents'>AAAA</span>",
            "U+0041",
            "loaded",
        ),
        (
            "<button style='display:contents'>AAAA</button>",
            "U+0041",
            "loaded",
        ),
    ] {
        assert_contents_subset(markup, range, expected);
    }
}
