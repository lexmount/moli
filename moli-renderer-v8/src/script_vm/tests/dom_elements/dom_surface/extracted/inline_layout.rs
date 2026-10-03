use super::*;

// Physical gaps verified in Chromium 145.0.7632.116 before adding assertions.
#[test]
fn inline_decorations_follow_visual_edges_of_natural_bidi_text() {
    for doctype in [
        "",
        r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd">"#,
        "<!doctype html>",
    ] {
        for direction in ["ltr", "rtl"] {
            for text in ["אב", "xאב", "אבx", "xאבx"] {
                for (decoration, left, right) in [
                    ("padding-left:20px", 20.0, 0.0),
                    ("padding-right:13px", 0.0, 13.0),
                    ("padding-left:20px;padding-right:13px", 20.0, 13.0),
                    ("border-left:7px solid;border-right:3px solid", 7.0, 3.0),
                ] {
                    let markup = format!(
                        r#"{doctype}<meta charset=utf-8><style>body{{margin:0}}#line{{direction:ltr;font:20px/30px monospace;width:300px}}</style><div id=line>a<span id=w style="direction:{direction};{decoration}">{text}</span>b</div>"#
                    );
                    let mut vm = new_parsed_test_vm("https://visual-inline-edges.test/", &markup);
                    let query = r#"JSON.stringify((()=>{
                        const word=document.getElementById('w'),range=document.createRange();
                        range.selectNodeContents(word);
                        const box=word.getBoundingClientRect(),text=range.getBoundingClientRect();
                        return [text.left-box.left,box.right-text.right,
                            document.getElementById('line').getBoundingClientRect().height,
                            word.textContent.length];
                    })())"#;
                    let first = vm.eval(query).unwrap();
                    let actual = serde_json::from_str::<[f64; 4]>(&first).unwrap();
                    let expected = [left, right, 30.0, text.chars().count() as f64];
                    for (actual, expected) in actual.into_iter().zip(expected) {
                        assert!(
                            (actual - expected).abs() < 0.02,
                            "{doctype}/{direction}/{text}/{decoration}: {first}"
                        );
                    }
                    publish_layout_for_test(&mut vm);
                    assert_eq!(
                        vm.eval(query).unwrap(),
                        first,
                        "{doctype}/{direction}/{text}/{decoration}"
                    );
                }
            }
        }
    }
}

// Relative geometry measured in Chromium 145.0.7632.116 in all three modes.
#[test]
fn atomic_inline_boxes_resolve_bidi_contexts_without_expanding_sibling_fragments() {
    for doctype in [
        "",
        r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd">"#,
        "<!doctype html>",
    ] {
        for context in [
            "normal",
            "embed",
            "bidi-override",
            "isolate",
            "isolate-override",
            "plaintext",
            "dir",
        ] {
            for direction in ["ltr", "rtl"] {
                for break_kind in ["br", "pre", "single"] {
                    for inside in [false, true] {
                        for tail_space in [false, true] {
                            if tail_space && break_kind != "pre" {
                                continue;
                            }
                            let white_space = if break_kind == "pre" {
                                "white-space:pre"
                            } else {
                                ""
                            };
                            let attributes = if context == "dir" {
                                format!(r#"dir={direction} style="{white_space}""#)
                            } else {
                                format!(
                                    r#"style="unicode-bidi:{context};direction:{direction};{white_space}""#
                                )
                            };
                            let prefix = match break_kind {
                                "br" => "x<br>",
                                "pre" => "x\n",
                                _ => "",
                            };
                            let atom = "<i id=atom></i>";
                            let markup = format!(
                                r#"{doctype}<style>body{{margin:0}}#line{{font:20px/30px monospace;width:300px}}#atom{{display:inline-block;width:10px;height:20px}}</style><div id=line><span {attributes}>{prefix}<span id=word>x</span>{}{}</span>{}</div>"#,
                                if tail_space { " " } else { "" },
                                if inside { atom } else { "" },
                                if inside { "" } else { atom },
                            );
                            let mut vm =
                                new_parsed_test_vm("https://atomic-inline-bidi.test/", &markup);
                            let query = r#"JSON.stringify((()=>{
                                const line=document.getElementById('line').getBoundingClientRect();
                                const word=document.getElementById('word'),range=document.createRange();
                                range.selectNodeContents(word);
                                const text=range.getBoundingClientRect(),box=word.getBoundingClientRect();
                                const atom=document.getElementById('atom').getBoundingClientRect();
                                return [line.height,text.left-line.left,text.right-line.left,
                                    atom.left-line.left,atom.width,box.left-line.left,box.width,
                                    word.getClientRects().length,range.getClientRects().length,word.textContent.length];
                            })())"#;
                            let first = vm.eval(query).unwrap();
                            let geometry = serde_json::from_str::<[f64; 10]>(&first).unwrap();
                            let text_width = geometry[2] - geometry[1];
                            let rtl_context =
                                direction == "rtl" && !matches!(context, "normal" | "plaintext");
                            let expected_text_left = if rtl_context {
                                (if inside { 10.0 } else { 0.0 })
                                    + if tail_space { text_width } else { 0.0 }
                            } else {
                                0.0
                            };
                            let expected_atom_left = if inside && rtl_context {
                                0.0
                            } else {
                                expected_text_left
                                    + text_width
                                    + if tail_space && !rtl_context {
                                        text_width
                                    } else {
                                        0.0
                                    }
                            };
                            let case = format!(
                                "{doctype} / {context} / {direction} / {break_kind} / inside={inside} / tail={tail_space}"
                            );
                            for (actual, expected) in [
                                (
                                    geometry[0],
                                    if break_kind == "single" { 30.0 } else { 60.0 },
                                ),
                                (geometry[1], expected_text_left),
                                (geometry[3], expected_atom_left),
                                (geometry[4], 10.0),
                                (geometry[5], geometry[1]),
                                (geometry[6], text_width),
                                (geometry[7], 1.0),
                                (geometry[8], 1.0),
                                (geometry[9], 1.0),
                            ] {
                                assert!(
                                    (actual - expected).abs() < 0.02,
                                    "{case}: {first}, expected {expected}, got {actual}"
                                );
                            }
                            publish_layout_for_test(&mut vm);
                            assert_eq!(vm.eval(query).unwrap(), first, "publication: {case}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn quirks_block_line_height_preserves_text_and_atomic_alignment() {
    for (doctype, quirks) in [
        ("", true),
        (
            r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd">"#,
            true,
        ),
        ("<!doctype html>", false),
    ] {
        let mut vm = new_parsed_test_vm(
            "https://quirks-block-line-height.test/",
            &format!(
                r#"{doctype}<style>
                  .line {{ font:50px/80px monospace; width:300px }}
                  .atomic {{ display:inline-block; width:10px; height:20px }}
                </style>
                <div id=only class=line><span class=atomic></span></div>
                <div id=text class=line>x<span class=atomic></span></div>
                <div id=smaller class=line><span style="font-size:10px;line-height:10px">x</span></div>
                <div id=aligned class=line><span class=atomic style="vertical-align:top"></span></div>
                <div id=spaced class=line> <span class=atomic></span> </div>"#
            ),
        );
        let query = r#"JSON.stringify(['only','text','smaller','aligned','spaced'].map(id => document.getElementById(id).getBoundingClientRect().height))"#;
        let expected = if quirks {
            "[20,80,10,20,20]"
        } else {
            "[80,80,80,80,80]"
        };
        assert_eq!(vm.eval(query).unwrap(), expected, "{doctype}");
        publish_layout_for_test(&mut vm);
        assert_eq!(vm.eval(query).unwrap(), expected, "paint: {doctype}");
    }
}

fn check_inline_line_height_modes(cases: &[(&str, &str, f64, f64)]) {
    for (doctype, quirks) in [
        ("", true),
        (
            r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd">"#,
            true,
        ),
        ("<!doctype html>", false),
    ] {
        let mut markup = format!(
            r#"{doctype}<style>.line{{font:50px/80px monospace;width:300px}}.atom{{display:inline-block;width:10px;height:20px}}em,i{{font-style:normal}}</style>"#
        );
        for (id, content, _, _) in cases {
            markup.push_str(&format!(r#"<div class=line id="{id}">{content}</div>"#));
        }
        let mut vm = new_parsed_test_vm("https://quirks-empty-inline.test/", &markup);
        let query = "JSON.stringify(Array.from(document.querySelectorAll('.line'), e=>e.getBoundingClientRect().height))";
        let result = vm.eval(query).unwrap();
        let heights: Vec<f64> = serde_json::from_str(&result).unwrap();
        for ((id, _, quirks_height, standards_height), actual) in cases.iter().zip(heights) {
            assert_eq!(
                actual,
                if quirks {
                    *quirks_height
                } else {
                    *standards_height
                },
                "{doctype} / {id}: {result}"
            );
        }
        publish_layout_for_test(&mut vm);
        assert_eq!(
            vm.eval(query).unwrap(),
            result,
            "publication uses the same line height rules"
        );
    }
}

#[test]
fn quirks_empty_inline_line_height_uses_text_and_fragment_edges() {
    check_inline_line_height_modes(&[
        ("direct", r#"<i class="atom"></i>"#, 20.0, 80.0),
        (
            "wrapped",
            r#"<span><i class="atom"></i></span>"#,
            20.0,
            80.0,
        ),
        (
            "nested",
            r#"<span><em><i class="atom"></i></em></span>"#,
            20.0,
            80.0,
        ),
        (
            "collapsed",
            "<span> \n <i class=\"atom\"></i> \n </span>",
            20.0,
            80.0,
        ),
        (
            "small_descendant_text",
            r#"<span><em style="font:10px/10px monospace">x</em></span>"#,
            10.0,
            80.0,
        ),
        (
            "own_text",
            r#"<span>x<i class="atom"></i></span>"#,
            80.0,
            80.0,
        ),
        (
            "between_atoms",
            r#"<span><i class="atom"></i> <i class="atom"></i></span>"#,
            80.0,
            80.0,
        ),
        (
            "preserved_space",
            r#"<span style="white-space:pre"> <i class="atom"></i> </span>"#,
            80.0,
            80.0,
        ),
        (
            "preserved_break",
            r#"<span style="white-space:pre-line">
</span>"#,
            80.0,
            80.0,
        ),
        ("br", r#"<span><br></span>"#, 80.0, 80.0),
        (
            "atomic_text",
            r#"<span><i style="display:inline-block;font:10px/10px monospace">x</i></span>"#,
            10.0,
            80.0,
        ),
        (
            "empty_next_to_atom",
            r#"<span></span><i class="atom"></i>"#,
            20.0,
            80.0,
        ),
        (
            "bidi",
            r#"<span dir=rtl><i class="atom"></i></span>"#,
            20.0,
            80.0,
        ),
        (
            "nested_small_atom",
            r#"<span style="font:100px/160px monospace"><em style="font:10px/10px monospace"><i class="atom"></i></em></span>"#,
            20.0,
            160.0,
        ),
    ]);
}

#[test]
fn quirks_empty_inline_line_height_distinguishes_border_padding_and_margin() {
    check_inline_line_height_modes(&[
        (
            "margin_only",
            r#"<span style="margin:1px"></span>"#,
            0.0,
            80.0,
        ),
        (
            "padding_block",
            r#"<span style="padding:7px 0"><i class="atom"></i></span>"#,
            20.0,
            80.0,
        ),
        (
            "border_block",
            r#"<span style="border-style:solid;border-width:7px 0"><i class="atom"></i></span>"#,
            20.0,
            80.0,
        ),
        (
            "padding_left",
            r#"<span style="padding-left:1px"><i class="atom"></i></span>"#,
            80.0,
            80.0,
        ),
        (
            "padding_right",
            r#"<span style="padding-right:1px"><i class="atom"></i></span>"#,
            80.0,
            80.0,
        ),
        (
            "border_left",
            r#"<span style="border-left:1px solid"><i class="atom"></i></span>"#,
            80.0,
            80.0,
        ),
        (
            "border_right",
            r#"<span style="border-right:1px solid"><i class="atom"></i></span>"#,
            80.0,
            80.0,
        ),
        (
            "hidden_border",
            r#"<span style="border-left:5px hidden"><i class="atom"></i></span>"#,
            20.0,
            80.0,
        ),
        (
            "none_border",
            r#"<span style="border-left:5px none"><i class="atom"></i></span>"#,
            20.0,
            80.0,
        ),
        (
            "cancelled_edge",
            r#"<span style="padding-left:1px;margin-left:-1px"><i class="atom"></i></span>"#,
            80.0,
            80.0,
        ),
        (
            "padding_zero_percent",
            r#"<span style="padding-left:0%"><i class="atom"></i></span>"#,
            20.0,
            80.0,
        ),
        (
            "padding_percent",
            r#"<span style="padding-left:1%"><i class="atom"></i></span>"#,
            80.0,
            80.0,
        ),
        (
            "margin_next_to_atom",
            r#"<span style="margin-left:1px"></span><i class="atom"></i>"#,
            20.0,
            80.0,
        ),
    ]);
}

#[test]
fn quirks_empty_inline_line_height_is_resolved_per_line() {
    check_inline_line_height_modes(&[
        (
            "multiple_lines",
            r#"<span><i class="atom"></i><br><i class="atom"></i></span>"#,
            40.0,
            160.0,
        ),
        (
            "text_first_line",
            r#"<span>x<br><i class="atom"></i></span>"#,
            100.0,
            160.0,
        ),
        (
            "text_last_line",
            r#"<span><i class="atom"></i><br>x</span>"#,
            100.0,
            160.0,
        ),
        (
            "border_multiple_lines",
            r#"<span style="border-left:1px solid"><i class="atom"></i><br><i class="atom"></i></span>"#,
            100.0,
            160.0,
        ),
        (
            "space_at_break",
            r#"<span><i class="atom"></i> <br><i class="atom"></i></span>"#,
            40.0,
            160.0,
        ),
        (
            "text_elsewhere",
            r#"<span>x</span><br><span><i class="atom"></i></span>"#,
            100.0,
            160.0,
        ),
        (
            "middle_atom",
            r#"<span><i class="atom" style="vertical-align:middle"></i></span>"#,
            20.0,
            80.0,
        ),
        (
            "top_wrapper",
            r#"<span style="vertical-align:top"><i class="atom"></i></span>"#,
            20.0,
            80.0,
        ),
        (
            "bottom_wrapper",
            r#"<span style="vertical-align:bottom"><i class="atom"></i></span>"#,
            20.0,
            80.0,
        ),
    ]);
}

#[test]
fn quirks_empty_inline_line_height_preserves_forced_empty_lines() {
    check_inline_line_height_modes(&[
        (
            "large_preserved_break",
            r#"<span style="font:20px/100px monospace;white-space:pre-line">
</span>"#,
            100.0,
            100.0,
        ),
        (
            "large_br",
            r#"<span style="font:20px/100px monospace"><br></span>"#,
            100.0,
            100.0,
        ),
        ("root_br", r#"<br>"#, 80.0, 80.0),
        (
            "large_nested_br",
            r#"<span style="font:100px/160px monospace"><em style="font:10px/10px monospace"><br></em></span>"#,
            10.0,
            160.0,
        ),
        (
            "br_own_font",
            r#"<br style="font:20px/100px monospace">"#,
            100.0,
            80.0,
        ),
        (
            "atom_large_br",
            r#"<span style="font:20px/100px monospace"><i class=atom></i><br></span>"#,
            20.0,
            100.0,
        ),
        (
            "atom_large_preserved_break",
            r#"<span style="font:20px/100px monospace;white-space:pre-line"><i class=atom></i>
</span>"#,
            20.0,
            100.0,
        ),
        (
            "atom_small_br",
            r#"<i class=atom></i><br style="font:10px/10px monospace">"#,
            20.0,
            80.0,
        ),
        (
            "margin_then_br",
            r#"<span style="margin-left:1px"></span><br style="font:20px/100px monospace">"#,
            100.0,
            80.0,
        ),
        (
            "padding_then_br",
            r#"<span style="padding-left:1px"></span><br style="font:20px/100px monospace">"#,
            80.0,
            80.0,
        ),
        (
            "padding_with_br",
            r#"<span style="padding-left:1px"><br style="font:20px/100px monospace"></span>"#,
            80.0,
            80.0,
        ),
        (
            "wrapped_auto_break",
            r#"<span><i class="atom" style="width:300px"></i> <i class="atom" style="width:300px"></i></span>"#,
            40.0,
            160.0,
        ),
        (
            "collapsed_large_trailing_space",
            r#"<i class="atom"></i><span style="font:100px/160px monospace"> </span>"#,
            20.0,
            160.0,
        ),
        (
            "border_empty",
            r#"<span style="border-right:1px solid"></span>"#,
            80.0,
            80.0,
        ),
    ]);
}

#[test]
fn quirks_empty_inline_line_height_collapses_spaces_after_wrapping() {
    check_inline_line_height_modes(&[
        (
            "wrapped_auto_break",
            r#"<span><i class="atom" style="width:300px"></i> <i class="atom" style="width:300px"></i></span>"#,
            40.0,
            160.0,
        ),
        (
            "wrapped_auto_break_preserved",
            r#"<span style="white-space:pre-wrap"><i class="atom" style="width:300px"></i> <i class="atom" style="width:300px"></i></span>"#,
            100.0,
            160.0,
        ),
        (
            "wrapped_auto_break_rtl",
            r#"<span dir=rtl><i class="atom" style="width:300px"></i> <i class="atom" style="width:300px"></i></span>"#,
            40.0,
            160.0,
        ),
        (
            "wrapped_auto_break_space_font",
            r#"<span><i class="atom" style="width:300px"></i><em style="font:100px/160px monospace"> </em><i class="atom" style="width:300px"></i></span>"#,
            40.0,
            240.0,
        ),
        (
            "wrapped_space_font_then_text",
            r#"<span><i class="atom" style="width:300px"></i><em style="font:100px/160px monospace"> </em>x</span>"#,
            100.0,
            240.0,
        ),
        (
            "wrapped_space_font_then_zero_text",
            r#"<span><i class="atom" style="width:300px"></i><em style="font:100px/160px monospace"> </em><b style="font-size:0;line-height:0">x</b></span>"#,
            20.0,
            240.0,
        ),
    ]);
}

#[test]
fn quirks_nested_break_line_height_uses_its_own_inline_box() {
    check_inline_line_height_modes(&[
        (
            "nested_br_padding",
            r#"<span style="font:10px/10px monospace;padding-left:1px"><span style="font:50px/80px monospace"><br></span></span>"#,
            80.0,
            80.0,
        ),
        (
            "nested_br_border",
            r#"<span style="font:10px/10px monospace;border-right:1px solid"><span style="font:50px/80px monospace"><br></span></span>"#,
            80.0,
            80.0,
        ),
        (
            "nested_br_text",
            r#"<span style="font:10px/10px monospace">x<span style="font:50px/80px monospace"><br></span></span>"#,
            80.0,
            80.0,
        ),
        (
            "nested_br_atom_sibling",
            r#"<i class="atom"></i><span><br></span>"#,
            80.0,
            80.0,
        ),
        (
            "nested_br_atom_in_parent",
            r#"<span><i class="atom"></i><span><br></span></span>"#,
            80.0,
            80.0,
        ),
        (
            "nested_br_atom_in_same_box",
            r#"<span><i class="atom"></i><br></span>"#,
            20.0,
            80.0,
        ),
        (
            "nested_br_descendant_text",
            r#"<span style="font:100px/160px monospace"><em style="font:10px/10px monospace">x</em><br></span>"#,
            10.0,
            160.0,
        ),
        (
            "nested_br_own_small_text",
            r#"<span style="font:10px/10px monospace">x<br style="font:50px/80px monospace"></span>"#,
            10.0,
            80.0,
        ),
        (
            "nested_preserved_break",
            r#"<span style="font:10px/10px monospace;padding-left:1px"><span style="font:50px/80px monospace;white-space:pre-line">
</span></span>"#,
            80.0,
            80.0,
        ),
    ]);
}

#[test]
fn quirks_wrapped_whitespace_line_height_counts_only_surviving_spaces() {
    check_inline_line_height_modes(&[
        (
            "wrapped_space",
            r#"<i class="atom"></i><span> </span><i class="atom"></i>"#,
            80.0,
            80.0,
        ),
        (
            "nested_wrapped_space",
            r#"<i class="atom"></i><span><em> </em></span><i class="atom"></i>"#,
            80.0,
            80.0,
        ),
        (
            "wrapped_space_large_font",
            r#"<i class="atom"></i><span style="font:100px/160px monospace"> </span><i class="atom"></i>"#,
            160.0,
            160.0,
        ),
        (
            "wrapped_space_small_font",
            r#"<i class="atom"></i><span style="font:10px/10px monospace"> </span><i class="atom"></i>"#,
            22.0,
            80.0,
        ),
        (
            "wrapped_space_rtl",
            r#"<span dir="rtl"><i class="atom"></i><span> </span><i class="atom"></i></span>"#,
            80.0,
            80.0,
        ),
        (
            "wrapped_space_preserved",
            r#"<i class="atom"></i><span style="white-space:pre"> </span><i class="atom"></i>"#,
            80.0,
            80.0,
        ),
        (
            "wrapped_space_leading",
            r#"<span> </span><i class="atom"></i>"#,
            20.0,
            80.0,
        ),
        (
            "wrapped_space_trailing",
            r#"<i class="atom"></i><span> </span>"#,
            20.0,
            80.0,
        ),
        (
            "wrapped_space_at_break",
            r#"<i class="atom"></i><span> </span><br><i class="atom"></i>"#,
            40.0,
            160.0,
        ),
        (
            "wrapped_space_after_wrap",
            r#"<i class="atom" style="width:300px"></i><span> </span><i class="atom" style="width:300px"></i>"#,
            40.0,
            160.0,
        ),
    ]);
}

#[test]
fn inline_wrapped_whitespace_preserves_sibling_spacing() {
    for doctype in [
        "",
        r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd">"#,
        "<!doctype html>",
    ] {
        let mut vm = new_parsed_test_vm(
            "https://inline-wrapped-whitespace.test/",
            &format!(
                r#"{doctype}<style>.line{{font:50px/80px monospace;width:300px}}.atom{{display:inline-block;width:10px;height:20px}}</style>
                <div id=plain class=line><i class=atom></i> <i class=atom></i></div>
                <div id=wrapped class=line><i class=atom></i><span> </span><i class=atom></i></div>
                <div id=nested class=line><i class=atom></i><span><span> </span></span><i class=atom></i></div>
                <div id=edges class=line><span> </span><i class=atom></i><span> </span></div>"#
            ),
        );
        let query = r#"JSON.stringify([
            ...['plain','wrapped','nested'].map(id => {
                const atoms = document.getElementById(id).querySelectorAll('.atom');
                return atoms[1].getBoundingClientRect().left - atoms[0].getBoundingClientRect().right;
            }),
            edges.querySelector('.atom').getBoundingClientRect().left - edges.getBoundingClientRect().left
        ])"#;
        let result = vm.eval(query).unwrap();
        let gaps: Vec<f64> = serde_json::from_str(&result).unwrap();
        assert!(gaps[0] > 0.0, "{doctype}: {result}");
        assert_eq!(gaps[1], gaps[0], "a wrapper must retain the space");
        assert_eq!(gaps[2], gaps[0], "nested wrappers must retain the space");
        assert_eq!(gaps[3], 0.0, "a leading wrapped space still collapses");
        publish_layout_for_test(&mut vm);
        assert_eq!(vm.eval(query).unwrap(), result, "paint: {doctype}");
    }
}

#[test]
fn inline_closing_edges_after_forced_breaks_contribute_to_their_own_fragment() {
    let cases = [
        (
            "left_border",
            r#"<span style="border-left:1px solid"><i class=atom></i><br></span><br>"#,
            160.0,
            160.0,
            true,
        ),
        (
            "right_border",
            r#"<span style="border-right:1px solid"><i class=atom></i><br></span><br>"#,
            160.0,
            160.0,
            true,
        ),
        (
            "right_padding",
            r#"<span style="padding-right:1px"><i class=atom></i><br></span><br>"#,
            160.0,
            160.0,
            true,
        ),
        (
            "terminal_break",
            r#"<span style="border-right:1px solid"><i class=atom></i><br></span>"#,
            80.0,
            80.0,
            true,
        ),
        (
            "preserved_break",
            "<span style=\"border-right:1px solid;white-space:pre\"><i class=atom></i>\n</span><br>",
            160.0,
            160.0,
            true,
        ),
        (
            "nested_closing_edges",
            r#"<span style="border-right:1px solid"><em><i class=atom></i><br></em></span><br>"#,
            160.0,
            160.0,
            true,
        ),
        (
            "nested_bidi_closing_edges",
            r#"<span style="padding-right:1px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span><br>"#,
            100.0,
            160.0,
            false,
        ),
        (
            "nested_bidi_terminal_padding",
            r#"<span style="padding-right:1px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span>"#,
            100.0,
            160.0,
            false,
        ),
        (
            "nested_bidi_terminal_border",
            r#"<span style="border-right:1px solid"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span>"#,
            100.0,
            160.0,
            false,
        ),
        (
            "nested_bidi_terminal_margin",
            r#"<span style="margin-right:1px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span>"#,
            20.0,
            160.0,
            false,
        ),
        (
            "multiple_fragments",
            r#"<span style="border-right:1px solid"><i class=atom></i><br><i class=atom></i><br></span><br>"#,
            180.0,
            240.0,
            false,
        ),
        (
            "opening_edge_after_break",
            r#"<span style="border-right:1px solid"><i class=atom></i><br><em></em></span><br>"#,
            100.0,
            160.0,
            false,
        ),
        (
            "undecorated",
            r#"<span><i class=atom></i><br></span><br>"#,
            100.0,
            160.0,
            false,
        ),
        (
            "right_margin",
            r#"<span style="margin-right:1px"><i class=atom></i><br></span><br>"#,
            100.0,
            160.0,
            false,
        ),
    ];
    for (doctype, quirks) in [
        ("", true),
        (
            r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd">"#,
            true,
        ),
        ("<!doctype html>", false),
    ] {
        let mut markup = format!(
            r#"{doctype}<style>.line{{font:50px/80px monospace;width:300px}}.atom{{display:inline-block;width:10px;height:20px}}em,i{{font-style:normal}}</style>"#
        );
        for (id, content, ..) in &cases {
            markup.push_str(&format!(r#"<div class=line id="{id}">{content}</div>"#));
        }
        let mut vm = new_parsed_test_vm("https://inline-closing-edge.test/", &markup);
        let query = r#"JSON.stringify(Array.from(document.querySelectorAll('.line'), e => {
            const line = e.getBoundingClientRect();
            const atom = e.querySelector('.atom').getBoundingClientRect();
            return [line.height, atom.top - line.top];
        }))"#;
        let result = vm.eval(query).unwrap();
        let geometries: Vec<[f64; 2]> = serde_json::from_str(&result).unwrap();
        let decorated_atom_top = geometries[0][1];
        for ((id, _, quirks_height, standards_height, decorated), actual) in
            cases.iter().zip(geometries)
        {
            assert_eq!(
                actual[0],
                if quirks {
                    *quirks_height
                } else {
                    *standards_height
                },
                "height: {doctype} / {id}: {result}"
            );
            assert_eq!(
                actual[1],
                if !quirks || *decorated {
                    decorated_atom_top
                } else {
                    0.0
                },
                "atom top: {doctype} / {id}"
            );
        }
        for (id, fragments) in [
            ("right_border", 1),
            ("right_padding", 1),
            ("terminal_break", 1),
            ("nested_bidi_closing_edges", 2),
            ("nested_bidi_terminal_padding", 2),
            ("nested_bidi_terminal_border", 2),
            ("nested_bidi_terminal_margin", 2),
        ] {
            assert_eq!(
                vm.eval(&format!(
                    "document.getElementById({id:?}).querySelector('span').getClientRects().length"
                ))
                .unwrap(),
                fragments.to_string(),
                "closing edge fragment count: {doctype} / {id}"
            );
        }
        let margin_fragment_query = r#"(() => {
            const rects = document.getElementById('nested_bidi_terminal_margin').querySelector('span').getClientRects();
            return rects[1].height / rects[0].height;
        })()"#;
        let margin_fragment_ratio = vm.eval(margin_fragment_query).unwrap();
        assert_eq!(margin_fragment_ratio, if quirks { "0" } else { "1" });
        publish_layout_for_test(&mut vm);
        assert_eq!(vm.eval(query).unwrap(), result, "paint: {doctype}");
        assert_eq!(
            vm.eval(margin_fragment_query).unwrap(),
            margin_fragment_ratio,
            "margin fragment paint: {doctype}"
        );
    }
}

#[test]
fn inline_used_closing_edges_preserve_standards_struts_and_ancestors() {
    check_inline_closing_fragment_modes(&[
        (
            "margin_tall",
            r#"<span style="line-height:160px;margin-right:1px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span>"#,
            20.0,
            320.0,
            0.0,
        ),
        (
            "padding_tall",
            r#"<span style="line-height:160px;padding-right:1px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span>"#,
            180.0,
            320.0,
            1.0,
        ),
        (
            "border_tall",
            r#"<span style="line-height:160px;border-right:1px solid"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span>"#,
            180.0,
            320.0,
            1.0,
        ),
        (
            "margin_nested",
            r#"<span style="line-height:200px"><span style="line-height:100px;margin-right:1px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span></span>"#,
            20.0,
            400.0,
            0.0,
        ),
    ]);
}

#[test]
fn standards_closing_inline_struts_do_not_depend_on_edge_sizes() {
    check_inline_line_height_modes(&[
        (
            "plain_tall_text",
            r#"<span style="line-height:160px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span>x"#,
            100.0,
            320.0,
        ),
        (
            "plain_tall_br",
            r#"<span style="line-height:160px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span><br>"#,
            100.0,
            320.0,
        ),
        (
            "plain_tall_terminal",
            r#"<span style="line-height:160px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span>"#,
            20.0,
            160.0,
        ),
        (
            "margin_tall_text",
            r#"<span style="line-height:160px;margin-right:1px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span>x"#,
            100.0,
            320.0,
        ),
        (
            "plain_nested_text",
            r#"<span style="line-height:200px"><span style="line-height:100px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span></span>x"#,
            100.0,
            400.0,
        ),
        (
            "plain_nested_br",
            r#"<span style="line-height:200px"><span style="line-height:100px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span></span><br>"#,
            100.0,
            400.0,
        ),
        (
            "plain_nested_terminal",
            r#"<span style="line-height:200px"><span style="line-height:100px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span></span>"#,
            20.0,
            200.0,
        ),
        (
            "plain_tall_atom",
            r#"<span style="line-height:160px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span><i class=atom></i>"#,
            40.0,
            320.0,
        ),
        (
            "plain_tall_zero_atom",
            r#"<span style="line-height:160px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span><i class=atom style="height:0"></i>"#,
            20.0,
            320.0,
        ),
        (
            "plain_tall_zero_text",
            r#"<span style="line-height:160px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span><b style="font-size:0;line-height:0">x</b>"#,
            20.0,
            320.0,
        ),
        (
            "plain_tall_zero_br",
            r#"<span style="line-height:160px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span><br style="line-height:0">"#,
            20.0,
            320.0,
        ),
    ]);
}

#[test]
fn quirks_closing_fragment_font_boxes_follow_line_content() {
    check_inline_closing_fragment_modes(&[
        (
            "margin_br",
            r#"<span style="margin-right:1px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span><br>"#,
            100.0,
            160.0,
            1.0,
        ),
        (
            "margin_text",
            r#"<span style="margin-right:1px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span>x"#,
            100.0,
            160.0,
            1.0,
        ),
        (
            "margin_atom",
            r#"<span style="margin-right:1px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span><i class=atom></i>"#,
            40.0,
            160.0,
            1.0,
        ),
        (
            "margin_zero_atom",
            r#"<span style="margin-right:1px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span><i class=atom style="height:0"></i>"#,
            20.0,
            160.0,
            1.0,
        ),
        (
            "margin_zero_text",
            r#"<span style="margin-right:1px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span><b style="font-size:0;line-height:0">x</b>"#,
            20.0,
            160.0,
            1.0,
        ),
        (
            "margin_zero_br",
            r#"<span style="margin-right:1px"><em style="unicode-bidi:embed;direction:rtl"><i class=atom></i><br></em></span><br style="line-height:0">"#,
            20.0,
            160.0,
            1.0,
        ),
    ]);
}

fn check_inline_closing_fragment_modes(cases: &[(&str, &str, f64, f64, f64)]) {
    for (doctype, quirks) in [
        ("", true),
        (
            r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd">"#,
            true,
        ),
        ("<!doctype html>", false),
    ] {
        let mut markup = format!(
            r#"{doctype}<style>.line{{font:50px/80px monospace;width:300px}}.atom{{display:inline-block;width:10px;height:20px}}em,i{{font-style:normal}}</style>"#
        );
        for (id, content, ..) in cases {
            markup.push_str(&format!(r#"<div class=line id="{id}">{content}</div>"#));
        }
        let mut vm = new_parsed_test_vm("https://inline-closing-fragment.test/", &markup);
        let query = r#"JSON.stringify(Array.from(document.querySelectorAll('.line'), e => {
            const rects = e.querySelector('span').getClientRects();
            return [e.getBoundingClientRect().height, rects.length, rects[1].height / rects[0].height];
        }))"#;
        let result = vm.eval(query).unwrap();
        let geometries: Vec<[f64; 3]> = serde_json::from_str(&result).unwrap();
        for ((id, _, quirks_height, standards_height, quirks_ratio), actual) in
            cases.iter().zip(geometries)
        {
            assert_eq!(
                actual,
                [
                    if quirks {
                        *quirks_height
                    } else {
                        *standards_height
                    },
                    2.0,
                    if quirks { *quirks_ratio } else { 1.0 },
                ],
                "closing fragment: {doctype} / {id}: {result}"
            );
        }
        publish_layout_for_test(&mut vm);
        assert_eq!(vm.eval(query).unwrap(), result, "paint: {doctype}");
    }
}

#[test]
fn inline_end_border_preserves_tall_atomic_content() {
    for doctype in ["", "<!doctype html>"] {
        let mut vm = new_parsed_test_vm(
            "https://inline-end-border-height.test/",
            &format!(
                r#"{doctype}<style>.line{{font:50px/80px monospace}}.atom{{display:inline-block;width:10px;height:200px}}</style><div class=line><span style="border-left:1px solid"><i class=atom></i></span></div><div class=line><span style="border-right:1px solid"><i class=atom></i></span></div>"#
            ),
        );
        let result = vm.eval("JSON.stringify(Array.from(document.querySelectorAll('.line'), e=>e.getBoundingClientRect().height))").unwrap();
        let heights: Vec<f64> = serde_json::from_str(&result).unwrap();
        assert!(heights[0] >= 200.0, "{result}");
        assert_eq!(
            heights[0], heights[1],
            "an end edge must retain earlier content bounds"
        );
    }
}

// Geometry measured in Chromium 145.0.7632.116, before and after publication.
#[test]
fn empty_inline_alignment_resolves_placement_separately_from_line_contribution() {
    let cases = [
        ("baseline", "<span id=b></span>", [20.0, 0.0], [80.0, 0.0]),
        (
            "raised",
            "<span id=b style=vertical-align:100px></span>",
            [20.0, -100.0],
            [180.0, -100.0],
        ),
        (
            "lowered",
            "<span id=b style=vertical-align:-100px></span>",
            [20.0, 100.0],
            [180.0, 100.0],
        ),
        (
            "percent",
            "<span id=b style=vertical-align:50%></span>",
            [20.0, -40.0],
            [120.0, -40.0],
        ),
        (
            "nested",
            "<span style=vertical-align:30px><span id=b style=vertical-align:100px></span></span>",
            [20.0, -130.0],
            [210.0, -130.0],
        ),
        (
            "nested_top",
            "<span style=vertical-align:top><span id=b style=vertical-align:100px></span><i class=atom></i></span>",
            [20.0, -100.0],
            [180.0, 0.0],
        ),
        (
            "content",
            "<span id=b style=vertical-align:100px><i class=atom></i></span>",
            [120.0, -100.0],
            [180.0, -100.0],
        ),
        (
            "zero_content",
            "<span id=b style=vertical-align:100px><i class=atom style=height:0></i></span>",
            [100.0, -100.0],
            [180.0, -100.0],
        ),
        (
            "text_top",
            "<span id=b style=vertical-align:text-top></span>",
            [46.0, -46.0],
            [91.0, 11.0],
        ),
        (
            "text_bottom",
            "<span id=b style=vertical-align:text-bottom></span>",
            [32.0, 12.0],
            [91.0, -11.0],
        ),
        (
            "top",
            "<span id=b style=vertical-align:top></span>",
            [20.0, -20.0],
            [80.0, 0.0],
        ),
        (
            "bottom",
            "<span id=b style=vertical-align:bottom></span>",
            [20.0, 0.0],
            [80.0, 0.0],
        ),
    ];
    for (doctype, quirks) in [
        ("", true),
        (
            r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd">"#,
            true,
        ),
        ("<!doctype html>", false),
    ] {
        for (id, content, quirks_geometry, standards_geometry) in cases {
            let markup = format!(
                r#"{doctype}<style>.line{{font:50px/80px monospace;width:300px}}.atom{{display:inline-block;width:10px;height:20px}}em,i{{font-style:normal}}</style><div id=line class=line><span id=a></span>{content}<i class=atom></i></div>"#
            );
            let mut vm = new_parsed_test_vm("https://inline-alignment.test/", &markup);
            let query = "JSON.stringify([line.getBoundingClientRect().height,b.getBoundingClientRect().top-a.getBoundingClientRect().top])";
            let expected = if quirks {
                quirks_geometry
            } else {
                standards_geometry
            };
            let first = vm.eval(query).unwrap();
            assert_eq!(
                serde_json::from_str::<[f64; 2]>(&first).unwrap(),
                expected,
                "{doctype} / {id}"
            );
            publish_layout_for_test(&mut vm);
            assert_eq!(vm.eval(query).unwrap(), first, "paint: {doctype} / {id}");
        }
    }
}

#[test]
fn forced_breaks_restore_ancestor_bidi_before_closing_inner_inline_boxes() {
    for (doctype, quirks) in [
        ("", true),
        (
            r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd">"#,
            true,
        ),
        ("<!doctype html>", false),
    ] {
        for bidi in [
            "normal",
            "embed",
            "isolate",
            "bidi-override",
            "isolate-override",
            "plaintext",
            "dir",
        ] {
            for direction in ["ltr", "rtl"] {
                for preserved in [false, true] {
                    let outer = if bidi == "dir" {
                        format!("dir={direction}")
                    } else {
                        format!(r#"style="unicode-bidi:{bidi};direction:{direction}""#)
                    };
                    let whitespace = if preserved { ";white-space:pre" } else { "" };
                    let newline = if preserved { "\n" } else { "<br>" };
                    let markup = format!(
                        r#"{doctype}<style>#line{{font:20px/30px monospace}}</style><div id=line><span {outer}><span style="font:30px/60px monospace{whitespace}">x{newline}</span><span id=second>x</span></span></div>"#
                    );
                    let mut vm =
                        new_parsed_test_vm("https://inline-bidi-forced-break.test/", &markup);
                    let query = "JSON.stringify([line.getBoundingClientRect().height,second.getBoundingClientRect().top-line.getBoundingClientRect().top])";
                    let expected = if quirks || bidi == "normal" {
                        [90.0, 63.0]
                    } else {
                        [120.0, 81.0]
                    };
                    let first = vm.eval(query).unwrap();
                    assert_eq!(
                        serde_json::from_str::<[f64; 2]>(&first).unwrap(),
                        expected,
                        "{doctype} / {bidi} / {direction} / pre={preserved}"
                    );
                    publish_layout_for_test(&mut vm);
                    assert_eq!(
                        vm.eval(query).unwrap(),
                        first,
                        "paint: {doctype} / {bidi} / {direction} / pre={preserved}"
                    );
                }
            }
        }
    }
}

#[test]
fn restored_bidi_controls_preserve_dom_range_offsets_across_lines() {
    for doctype in [
        "",
        r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd">"#,
        "<!doctype html>",
    ] {
        for bidi in ["embed", "isolate-override"] {
            for direction in ["ltr", "rtl"] {
                let markup = format!(
                    r#"{doctype}<style>#line{{font:20px/30px monospace}}</style><div id=line><span style="unicode-bidi:{bidi};direction:{direction}"><span id=inner style="font:30px/60px monospace;white-space:pre">x
x</span></span></div>"#
                );
                let mut vm = new_parsed_test_vm("https://bidi-source-mapping.test/", &markup);
                let query = r#"(() => {
                    const a=document.createRange(), b=document.createRange();
                    a.setStart(inner.firstChild,0); a.setEnd(inner.firstChild,1);
                    b.setStart(inner.firstChild,2); b.setEnd(inner.firstChild,3);
                    const ar=a.getBoundingClientRect(), br=b.getBoundingClientRect();
                    return JSON.stringify([line.getBoundingClientRect().height,br.top-ar.top,br.width/ar.width,a.getClientRects().length,b.getClientRects().length]);
                })()"#;
                let first = vm.eval(query).unwrap();
                assert_eq!(first, "[120,60,1,1,1]", "{doctype} / {bidi} / {direction}");
                publish_layout_for_test(&mut vm);
                assert_eq!(
                    vm.eval(query).unwrap(),
                    first,
                    "paint: {doctype} / {bidi} / {direction}"
                );
            }
        }
    }
}

#[test]
fn html_direction_hints_and_bidi_contexts_stay_in_the_style_cascade() {
    for doctype in [
        "",
        r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd">"#,
        "<!doctype html>",
    ] {
        let markup = format!(
            r#"{doctype}<style>.override{{direction:ltr;unicode-bidi:normal}}</style><span id=ltr dir=ltr></span><span id=rtl dir=RTL></span><span id=invalid dir=" rtl"></span><span id=override dir=rtl style="direction:ltr;unicode-bidi:normal"></span>"#
        );
        let mut vm = new_parsed_test_vm("https://html-bidi-style.test/", &markup);
        let query = r#"JSON.stringify(['ltr','rtl','invalid','override'].map(id=>{
            const s=getComputedStyle(document.getElementById(id));
            return [s.direction,s.unicodeBidi];
        }))"#;
        let first = vm.eval(query).unwrap();
        assert_eq!(
            first, r#"[["ltr","isolate"],["rtl","isolate"],["ltr","normal"],["ltr","normal"]]"#,
            "{doctype}"
        );
        publish_layout_for_test(&mut vm);
        assert_eq!(vm.eval(query).unwrap(), first, "paint: {doctype}");
    }
}
