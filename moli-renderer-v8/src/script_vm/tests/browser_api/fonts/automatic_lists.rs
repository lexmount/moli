use super::automatic_controls::font_usage_vm;
use super::selection_order::order_font_data;
use super::*;

#[test]
fn automatic_fonts_default_markers_use_rendered_list_types_and_counters() {
    let (_, wide) = order_font_data();
    for (markup, css, range, expected) in [
        (
            "<ol><li class='target'>body</li></ol>",
            "",
            "U+0031",
            "loaded",
        ),
        (
            "<ul><li class='target'>body</li></ul>",
            "",
            "U+2022",
            "loaded",
        ),
        (
            "<ol start='26' style='list-style-type:lower-alpha'><li class='target'>body</li></ol>",
            "",
            "U+007A",
            "loaded",
        ),
        (
            "<ol reversed><li class='target'>body</li><li>body</li></ol>",
            "",
            "U+0032",
            "loaded",
        ),
        (
            "<ol reversed start='5'><li>body</li><li class='target'>body</li></ol>",
            "",
            "U+0034",
            "loaded",
        ),
        (
            "<ol start='4'><li value='8'>body</li><li class='target'>body</li></ol>",
            "",
            "U+0039",
            "loaded",
        ),
        (
            "<ol start='-2'><li class='target'>body</li></ol>",
            "",
            "U+002D",
            "loaded",
        ),
        (
            "<ol><li hidden>body</li><li class='target'>body</li></ol>",
            "",
            "U+0031",
            "loaded",
        ),
        (
            "<ol><li style='content-visibility:hidden'>body</li><li class='target'>body</li></ol>",
            "",
            "U+0032",
            "loaded",
        ),
        (
            "<ol><li>outer<ol><li>nested</li></ol></li><li class='target'>body</li></ol>",
            "",
            "U+0032",
            "loaded",
        ),
        (
            "<ol><div style='display:contents'><li class='target'>body</li></div></ol>",
            "",
            "U+0031",
            "loaded",
        ),
        (
            "<div class='target' style='display:list-item;list-style-type:upper-alpha'>body</div>",
            "",
            "U+0041",
            "loaded",
        ),
        (
            "<ol><li>body</li><div class='target' value='8' style='display:list-item'>body</div></ol>",
            "",
            "U+0032",
            "loaded",
        ),
        (
            "<ol style='list-style-type:none'><li class='target'>body</li></ol>",
            "",
            "U+0031",
            "unloaded",
        ),
        (
            "<ol><li class='target'>body</li></ol>",
            ".target::marker{content:none}",
            "U+0031",
            "unloaded",
        ),
        (
            "<ol><li class='target'>body</li></ol>",
            ".target::marker{display:contents}",
            "U+0031",
            "loaded",
        ),
        (
            "<ol start='4'><li class='target'>body</li></ol>",
            "",
            "U+0031",
            "unloaded",
        ),
    ] {
        let (mut vm, mut completions) = font_usage_vm();
        let markup = serde_json::to_string(markup).unwrap();
        let css = serde_json::to_string(&format!(".target::marker{{font-family:MarkerFace}}{css}"))
            .unwrap();
        vm.eval(&format!(r#"
            const style=document.createElement('style');style.textContent={css};document.head.append(style);
            document.body.innerHTML={markup};
            globalThis.face=new FontFace('MarkerFace','url(data:font/ttf;base64,{wide})',{{unicodeRange:'{range}'}});
            document.fonts.add(face);
        "#)).unwrap();
        vm.reconcile_document_font_resources();
        vm.eval("document.fonts.ready.then(()=>globalThis.ready=true)")
            .unwrap();
        complete_local_font_resources(&mut vm, &mut completions);
        assert_eq!(
            vm.eval("face.status").unwrap(),
            expected,
            "{markup}/{css}/{range}"
        );
        if css.contains("::marker{display:contents}") {
            assert_eq!(
                vm.eval("getComputedStyle(document.querySelector('.target'),'::marker').display")
                    .unwrap(),
                "inline"
            );
        }
        assert_eq!(vm.eval("globalThis.ready===true").unwrap(), "true");
        assert_eq!(vm.layout_pass_observability_for_test().1, 0);
    }
}

#[test]
fn automatic_fonts_default_markers_count_slotted_items_in_rendered_order() {
    let (mut vm, mut completions) = font_usage_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        const style=document.createElement('style');
        style.textContent='.target::marker{{font-family:MarkerFace}}';document.head.append(style);
        const host=document.createElement('div');
        host.innerHTML='<li>first</li><li class="target" style="list-style-type:decimal">second</li>';
        const shadow=host.attachShadow({{mode:'open'}});
        shadow.innerHTML='<style>slot {{display:contents}}</style><ol start="8"><slot></slot></ol>';
        document.body.append(host);
        globalThis.face=new FontFace('MarkerFace','url(data:font/ttf;base64,{wide})',{{unicodeRange:'U+0039'}});
        document.fonts.add(face);
    "#)).unwrap();
    vm.reconcile_document_font_resources();
    complete_local_font_resources(&mut vm, &mut completions);
    assert_eq!(vm.eval("face.status").unwrap(), "loaded");
    assert_eq!(vm.layout_pass_observability_for_test().1, 0);
}

fn assert_automatic_marker_number(markup: &str, css: &str, expected: u8) {
    let (mut vm, mut completions) = font_usage_vm();
    let (_, wide) = order_font_data();
    let markup = serde_json::to_string(markup).unwrap();
    let css =
        serde_json::to_string(&format!(".target::marker{{font-family:MarkerFace}}{css}")).unwrap();
    vm.eval(&format!(r#"
        const style=document.createElement('style');style.textContent={css};document.head.append(style);
        document.body.innerHTML={markup};
        const source='url(data:font/ttf;base64,{wide})';
        globalThis.markerFaces=[1,2,3,4].map(number=>{{
            const face=new FontFace('MarkerFace',source,{{unicodeRange:'U+003'+number}});
            document.fonts.add(face);
            return face;
        }});
    "#)).unwrap();
    vm.reconcile_document_font_resources();
    vm.eval("document.fonts.ready.then(()=>globalThis.ready=true)")
        .unwrap();
    complete_local_font_resources(&mut vm, &mut completions);
    let expected_statuses = (1..=4)
        .map(|number| {
            if number == expected {
                "loaded"
            } else {
                "unloaded"
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(
        vm.eval("markerFaces.map(face=>face.status).join(',')")
            .unwrap(),
        expected_statuses,
        "{markup}/{css}"
    );
    assert_eq!(vm.eval("globalThis.ready===true").unwrap(), "true");
    assert_eq!(vm.layout_pass_observability_for_test().1, 0);
}

#[test]
fn automatic_fonts_marker_counters_skip_filtered_column_group_items() {
    let css = ".host::before,.host::after{content:'x';display:list-item}";
    for (host, expected) in [
        (
            "<li class='host' style='display:table-column-group'></li>",
            1,
        ),
        (
            "<div class='host' style='display:table-column-group'><li>ignored</li></div>",
            1,
        ),
        (
            "<div class='host' style='display:table-column-group'><div style='display:contents'><li>ignored</li></div></div>",
            1,
        ),
        (
            "<div class='host' style='display:table-column-group'><div style='display:table-column'></div></div>",
            1,
        ),
        ("<li class='host' style='display:contents'></li>", 3),
        ("<li class='host' style='display:block'></li>", 3),
        ("<li class='host' style='display:list-item'></li>", 2),
    ] {
        assert_automatic_marker_number(
            &format!("<ol>{host}<li class='target'>body</li></ol>"),
            css,
            expected,
        );
        assert_automatic_marker_number(
            &format!("<ol reversed><li class='target'>body</li>{host}</ol>"),
            css,
            expected,
        );
    }
}

#[test]
fn automatic_fonts_default_markers_count_generated_items_in_box_order() {
    for (markup, css, expected) in [
        (
            "<ol><li class='target'>body</li></ol>",
            "ol::before{content:'prefix';display:list-item}",
            2,
        ),
        (
            "<ol><li class='target'>body</li></ol>",
            "ol::after{content:'suffix';display:list-item}",
            1,
        ),
        (
            "<ol><li class='target'>body</li></ol>",
            "ol::before,ol::after{content:'x';display:list-item}",
            2,
        ),
        (
            "<ol><li class='target'>body</li></ol>",
            "ol::before{content:'';display:list-item}",
            2,
        ),
        (
            "<ol><div></div><li class='target'>body</li></ol>",
            "div::before,div::after{content:'x';display:list-item}",
            3,
        ),
        (
            "<ol><div style='display:contents'></div><li class='target'>body</li></ol>",
            "div::before,div::after{content:'x';display:list-item}",
            3,
        ),
        (
            "<ol><li>first</li><li class='target'>body</li></ol>",
            "li::before,li::after{content:'x';display:list-item}",
            2,
        ),
        (
            "<ol><li>first<ol><li>nested</li></ol></li><li class='target'>body</li></ol>",
            "ol ol::before,ol ol::after{content:'x';display:list-item}",
            2,
        ),
    ] {
        assert_automatic_marker_number(markup, css, expected);
    }
}

#[test]
fn automatic_fonts_reversed_markers_count_generated_items() {
    for (attributes, css, expected) in [
        (
            "reversed",
            "ol::after{content:'suffix';display:list-item}",
            2,
        ),
        (
            "reversed",
            "ol::before{content:'prefix';display:list-item}",
            1,
        ),
        (
            "reversed",
            "ol::before,ol::after{content:'x';display:list-item}",
            2,
        ),
        (
            "reversed start='5'",
            "ol::before{content:'prefix';display:list-item}",
            4,
        ),
    ] {
        assert_automatic_marker_number(
            &format!("<ol {attributes}><li class='target'>body</li></ol>"),
            css,
            expected,
        );
    }
}

#[test]
fn automatic_fonts_default_markers_ignore_suppressed_generated_items() {
    for attributes in ["", "reversed"] {
        for declarations in [
            "content:none;display:list-item",
            "content:normal;display:list-item",
            "content:'x';display:none",
            "content:'x';display:contents",
            "content:'x';display:block",
        ] {
            assert_automatic_marker_number(
                &format!("<ol {attributes}><li class='target'>body</li></ol>"),
                &format!("ol::before,ol::after{{{declarations}}}"),
                1,
            );
        }
    }
}

#[test]
fn automatic_fonts_marker_counters_ignore_leaf_host_generated_items() {
    assert_automatic_marker_number(
        "<ol><br><li class='target'>body</li></ol>",
        "br::before{content:'x';display:list-item}",
        1,
    );
    for leaf in [
        "<br class='leaf'>",
        "<div class='leaf' style='display:table-column'></div>",
        "<div class='leaf' style='display:table-column'><li>ignored child</li></div>",
        "<img class='leaf'>",
        "<input class='leaf'>",
        "<textarea class='leaf'></textarea>",
        "<select class='leaf'><option>label</option></select>",
        "<progress class='leaf'></progress>",
        "<meter class='leaf'></meter>",
        "<canvas class='leaf'></canvas>",
        "<iframe class='leaf'></iframe>",
        "<embed class='leaf'>",
        "<audio class='leaf'></audio>",
        "<video class='leaf'></video>",
        "<svg class='leaf'></svg>",
    ] {
        for reversed in [false, true] {
            let markup = if reversed {
                format!("<ol reversed><li class='target'>body</li>{leaf}</ol>")
            } else {
                format!("<ol>{leaf}<li class='target'>body</li></ol>")
            };
            assert_automatic_marker_number(
                &markup,
                ".leaf::before,.leaf::after{content:'x';display:list-item}",
                1,
            );
        }
    }
}

#[test]
fn automatic_fonts_marker_counters_follow_host_display_and_fallback_rules() {
    for (markup, expected) in [
        ("<br class='host' style='display:list-item'>", 2),
        ("<br class='host' style='display:contents'>", 1),
        ("<wbr class='host'>", 3),
        ("<wbr class='host' style='display:contents'>", 1),
        ("<button class='host'>label</button>", 3),
        ("<object class='host'>fallback</object>", 3),
        (
            "<object class='host' style='display:contents'>fallback</object>",
            1,
        ),
        ("<div class='host' style='display:contents'></div>", 3),
        ("<div class='host' style='display:none'></div>", 1),
    ] {
        assert_automatic_marker_number(
            &format!("<ol>{markup}<li class='target'>body</li></ol>"),
            ".host::before,.host::after{content:'x';display:list-item}",
            expected,
        );
    }
}
