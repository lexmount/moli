use super::selection_order::{order_font_data, order_vm};
use super::*;
use crate::protocol_types::OptionalResourceFetchMask;

pub(super) fn font_usage_vm() -> StandaloneScriptVmHarness {
    let mut vm = order_vm();
    vm.current_main_document_resource_loader()
        .unwrap()
        .request_client()
        .set_optional_resource_fetch_mask(OptionalResourceFetchMask::FONT);
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
    vm
}

#[test]
fn automatic_fonts_select_labels_match_the_rendered_control_rows() {
    let (_, wide) = order_font_data();
    for (attributes, options, setup, expected) in [
        ("", "<option selected label='AAAA'></option>", "", "loaded"),
        (
            "size='3'",
            "<option selected label='BBBB'></option><option label='AAAA'></option>",
            "",
            "loaded",
        ),
        ("multiple", "<option label='AAAA'></option>", "", "loaded"),
        (
            "",
            "<option selected label='BBBB'></option><option label='AAAA'></option>",
            "",
            "unloaded",
        ),
        (
            "",
            "<option selected label='BBBB'>AAAA</option>",
            "",
            "unloaded",
        ),
        (
            "size='3'",
            "<option label='BBBB'>AAAA</option>",
            "",
            "unloaded",
        ),
        (
            "",
            "<option selected label='AAAA' style='font-family:serif'></option>",
            "",
            "loaded",
        ),
        (
            "",
            "<option selected label='BBBB'></option><option label='AAAA'></option>",
            "control.selectedIndex=1",
            "loaded",
        ),
        (
            "",
            "<option selected label='BBBB'></option>",
            "control.options[0].label='AAAA'",
            "loaded",
        ),
        (
            "size='1' multiple",
            "<option label='AAAA'></option>",
            "",
            "unloaded",
        ),
        (
            "size='0' multiple",
            "<option label='AAAA'></option>",
            "",
            "loaded",
        ),
        ("", "<option selected> A  A </option>", "", "loaded"),
        (
            "",
            "<optgroup label='AAAA'><option selected label='BBBB'></option></optgroup>",
            "",
            "unloaded",
        ),
    ] {
        let mut vm = font_usage_vm();
        let markup = serde_json::to_string(&format!(
            "<select id='control' style='font-family:SelectFace' {attributes}>{options}</select>"
        ))
        .unwrap();
        vm.eval(&format!(r#"
            document.body.innerHTML={markup};
            {setup};
            globalThis.face=new FontFace('SelectFace','url(data:font/ttf;base64,{wide})',{{unicodeRange:'U+0041'}});
            document.fonts.add(face);
        "#)).unwrap();
        vm.reconcile_document_font_resources();
        vm.eval("document.fonts.ready.then(()=>globalThis.ready=true)")
            .unwrap();
        run_font_tasks(&mut vm);
        assert_eq!(
            vm.eval("face.status").unwrap(),
            expected,
            "{attributes}/{options}/{setup}"
        );
        assert_eq!(vm.eval("globalThis.ready===true").unwrap(), "true");
        assert_eq!(vm.layout_pass_observability_for_test().1, 0);
    }
}

#[test]
fn automatic_fonts_use_the_visible_placeholder_pseudo_style() {
    let (_, wide) = order_font_data();
    for tag in ["input", "textarea"] {
        for value in ["", "AAAA"] {
            let mut vm = font_usage_vm();
            vm.eval(&format!(r#"
                globalThis.placeholderFace=new FontFace('PlaceholderFace','url(data:font/ttf;base64,{wide})');
                globalThis.valueFace=new FontFace('ValueFace','url(data:font/ttf;base64,{wide})');
                document.fonts.add(placeholderFace);document.fonts.add(valueFace);
                const style=document.createElement('style');
                style.textContent='input::placeholder,textarea::placeholder {{font-family:PlaceholderFace}}';
                document.head.append(style);
                const control=document.createElement('{tag}');
                control.style.fontFamily='ValueFace';control.placeholder='AAAA';control.value='{value}';
                document.body.append(control);
            "#)).unwrap();
            vm.reconcile_document_font_resources();
            vm.eval("document.fonts.ready.then(()=>globalThis.ready=true)")
                .unwrap();
            run_font_tasks(&mut vm);
            assert_eq!(
                vm.eval("[placeholderFace.status,valueFace.status].join(',')")
                    .unwrap(),
                if value.is_empty() {
                    "loaded,unloaded"
                } else {
                    "unloaded,loaded"
                },
                "{tag}, value={value:?}"
            );
            assert_eq!(vm.eval("String(ready)").unwrap(), "true");
            assert_eq!(vm.layout_pass_observability_for_test().1, 0);
        }
    }
}

#[test]
fn automatic_fonts_include_generated_list_marker_text() {
    let (_, wide) = order_font_data();
    for (display, content, expected) in [
        ("list-item", "'A'", "loaded"),
        ("block", "'A'", "unloaded"),
        ("list-item", "none", "unloaded"),
    ] {
        let mut vm = font_usage_vm();
        let css = serde_json::to_string(&format!(
            "#marker {{display:{display}}} #marker::marker {{content:{content};font-family:MarkerFace}}"
        ))
        .unwrap();
        vm.eval(&format!(r#"
            globalThis.face=new FontFace('MarkerFace','url(data:font/ttf;base64,{wide})');document.fonts.add(face);
            const style=document.createElement('style');
            style.textContent={css};
            document.head.append(style);
            const item=document.createElement('div');item.id='marker';item.textContent='body';document.body.append(item);
        "#)).unwrap();
        vm.reconcile_document_font_resources();
        run_font_tasks(&mut vm);
        assert_eq!(
            vm.eval("face.status").unwrap(),
            expected,
            "{display}/{content}"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, 0);
    }
}

#[test]
fn automatic_fonts_include_default_form_labels_and_skip_nontext_controls() {
    let (_, wide) = order_font_data();
    for (kind, value, range, expected) in [
        ("submit", "", "U+0053", "loaded"),
        ("reset", "", "U+0052", "loaded"),
        ("file", "", "U+0043", "loaded"),
        ("submit", "AAAA", "U+0041", "loaded"),
        ("reset", "AAAA", "U+0052", "unloaded"),
        ("button", "", "U+0041", "unloaded"),
        ("checkbox", "AAAA", "U+0041", "unloaded"),
        ("radio", "AAAA", "U+0041", "unloaded"),
        ("hidden", "AAAA", "U+0041", "unloaded"),
        ("image", "AAAA", "U+0041", "unloaded"),
    ] {
        let mut vm = font_usage_vm();
        vm.eval(&format!(r#"
            globalThis.face=new FontFace('ControlFace','url(data:font/ttf;base64,{wide})',{{unicodeRange:'{range}'}});
            document.fonts.add(face);
            const control=document.createElement('input');control.type='{kind}';control.value='{value}';
            control.style.fontFamily='ControlFace';document.body.append(control);
        "#)).unwrap();
        vm.reconcile_document_font_resources();
        run_font_tasks(&mut vm);
        assert_eq!(
            vm.eval("face.status").unwrap(),
            expected,
            "{kind}, value={value:?}, {range}"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, 0);
    }
}
