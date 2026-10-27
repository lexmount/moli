use super::automatic_controls::font_usage_vm;
use super::selection_order::order_font_data;
use super::*;

fn assert_participating_text(markup: &str, range: &str, expected: &str) {
    let mut vm = font_usage_vm();
    let (_, wide) = order_font_data();
    let markup = serde_json::to_string(markup).unwrap();
    vm.eval(&format!(r#"
        globalThis.face=new FontFace('ParticipationFace','url(data:font/ttf;base64,{wide})',{{unicodeRange:'{range}'}});
        document.fonts.add(face);
        const style=document.createElement('style');
        style.textContent='input,textarea,select,button {{font-family:ParticipationFace}} .generated::before,.generated::after {{content:"A"}}';
        document.head.append(style);
        const container=document.createElement('div');container.style.fontFamily='ParticipationFace';
        container.innerHTML={markup};document.body.append(container);
    "#)).unwrap();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("face.status").unwrap(), expected, "{markup}");
    vm.eval("document.fonts.ready.then(()=>globalThis.ready=true)")
        .unwrap();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("globalThis.ready===true").unwrap(), "true");
    assert_eq!(vm.layout_pass_observability_for_test().1, 0);
}

#[test]
fn automatic_fonts_content_visibility_respects_property_applicability() {
    for (display, hidden) in [
        ("inline", "loaded"),
        ("contents", "loaded"),
        ("block", "unloaded"),
        ("inline-block", "unloaded"),
        ("flex", "unloaded"),
        ("grid", "unloaded"),
        ("table", "loaded"),
        ("table-cell", "unloaded"),
    ] {
        for (visibility, expected) in [("hidden", hidden), ("visible", "loaded")] {
            assert_participating_text(
                &format!("<div style='display:{display};content-visibility:{visibility}'>A</div>"),
                "U+0041",
                expected,
            );
        }
    }
}

#[test]
fn automatic_fonts_skip_leaf_fallback_and_pseudo_text_but_keep_control_labels() {
    for (markup, expected) in [
        ("<canvas>A</canvas>", "unloaded"),
        ("<canvas class='generated'></canvas>", "unloaded"),
        (
            "<canvas class='generated' style='display:list-item'>A</canvas>",
            "unloaded",
        ),
        ("<canvas style='display:contents'>A</canvas>", "unloaded"),
        ("<canvas><input value='A'></canvas>", "unloaded"),
        ("<progress class='generated'>A</progress>", "unloaded"),
        ("<meter class='generated'>A</meter>", "unloaded"),
        ("<audio class='generated'>A</audio>", "unloaded"),
        ("<video class='generated'>A</video>", "unloaded"),
        ("<iframe class='generated'>A</iframe>", "unloaded"),
        ("<div style='display:table-column'>A</div>", "unloaded"),
        (
            "<div class='generated' style='display:table-column-group'><div>A</div></div>",
            "unloaded",
        ),
        ("<object>A</object>", "loaded"),
        ("<object style='display:contents'>A</object>", "unloaded"),
        ("<button>A</button>", "loaded"),
        ("<button style='display:contents'>A</button>", "loaded"),
        ("<span style='display:contents'>A</span>", "loaded"),
        ("<input value='A'>", "loaded"),
        ("<textarea>A</textarea>", "loaded"),
        ("<select><option label='A'></option></select>", "loaded"),
    ] {
        assert_participating_text(markup, "U+0041", expected);
    }
}

#[test]
fn automatic_fonts_hidden_boxes_preserve_outer_text_boundaries() {
    for (markup, expected) in [
        (
            "<span style='text-transform:capitalize'>a<div style='content-visibility:hidden'>x</div>b</span>",
            "loaded",
        ),
        (
            "<span style='text-transform:capitalize'>a<div style='display:none'>x</div>b</span>",
            "unloaded",
        ),
        (
            "<span style='text-transform:capitalize'>a<span style='display:inline-block;content-visibility:hidden'>x</span>b</span>",
            "unloaded",
        ),
    ] {
        assert_participating_text(markup, "U+0042", expected);
    }
}
