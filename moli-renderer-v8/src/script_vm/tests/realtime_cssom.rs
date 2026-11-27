use super::*;
use moli_page_types::LayoutPolicy;
use std::fmt::Write as _;

fn realtime_vm(markup: &str) -> StandaloneScriptVmHarness {
    let mut vm = new_parsed_test_vm("https://realtime-cssom.test/", markup);
    vm.set_layout_policy(LayoutPolicy::Realtime);
    vm
}

#[test]
fn realtime_cssom_retained_declaration_reads_current_used_sizes() {
    let mut vm = realtime_vm(
        "<!doctype html><div id=container style='width:200px'><div id=target style='width:50%;height:10px'></div></div>",
    );
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "const style=getComputedStyle(target);const values=[style.width];\
             container.style.width='400px';values.push(style.width,style.getPropertyValue('width'),style.inlineSize);\
             target.style.width='75%';values.push(style.getPropertyValue('width'));JSON.stringify(values)",
        )
        .unwrap(),
        r#"["100px","200px","200px","200px","300px"]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 3);
}

#[test]
fn realtime_size_container_queries_follow_nested_content_boxes() {
    let mut vm = realtime_vm(
        r#"<!doctype html><style>
        #frame { width: 400px }
        #outer { container: shell / inline-size; box-sizing: border-box;
                 width: 100%; padding: 20px; border: 5px solid; transform: scale(.5) }
        #inner { container: card / inline-size; width: 150px }
        #target { width: 20px }
        @container shell (width >= 300px) { #inner { width: 320px } }
        @container card (width >= 300px) { #target { width: 110px } }
        </style><div id=frame><div id=outer><div id=inner><div id=target></div></div></div></div>"#,
    );
    assert_eq!(
        vm.eval(
            r#"const sizes = () => [inner.offsetWidth, target.offsetWidth];
            const initial = sizes();
            frame.style.width = '340px';
            const narrower = sizes();
            frame.style.width = '400px';
            JSON.stringify([initial, narrower, sizes()])"#,
        )
        .unwrap(),
        "[[320,110],[150,20],[320,110]]"
    );
}

#[test]
fn realtime_cssom_size_container_query_refreshes_retained_color_and_display() {
    let mut vm = realtime_vm(
        r#"<!doctype html><style>
        #target { color: red; display: block }
        #unrelated { width: 10cqw; color: blue }
        @container (width >= 300px) { #target { color: green; display: flex } }
        </style><div id=box style='container-type:inline-size;width:200px'>
        <div id=target></div></div><div id=unrelated></div>"#,
    );
    assert_eq!(
        vm.eval("const held = getComputedStyle(target); box.offsetWidth")
            .unwrap(),
        "200"
    );
    let builds = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval("box.style.width = '400px'; getComputedStyle(unrelated).color")
            .unwrap(),
        "rgb(0, 0, 255)"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    assert_eq!(vm.eval("held.color").unwrap(), "rgb(0, 128, 0)");
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
    assert_eq!(
        vm.eval("box.style.width = '200px'; held.getPropertyValue('display')")
            .unwrap(),
        "block"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 2);
}

#[test]
fn realtime_cssom_container_query_refreshes_retained_property_names() {
    let mut vm = realtime_vm(
        r#"<!doctype html><style>
        @container (width >= 300px) { #target { --wide: yes } }
        </style><div id=box style="container-type:inline-size;width:200px">
        <div id=target></div></div>"#,
    );
    assert_eq!(
        vm.eval(
            r#"const held = getComputedStyle(target);
            void box.offsetWidth;
            const narrowCount = held.length;
            box.style.width = '400px';
            const lengthDifference = held.length - narrowCount;
            box.style.width = '200px';
            const missing = held.item(narrowCount);
            box.style.width = '400px';
            const item = held.item(narrowCount);
            box.style.width = '200px';
            const hasIndex = narrowCount in held;
            box.style.width = '400px';
            const indexed = held[narrowCount];
            box.style.width = '200px';
            const enumerated = Object.keys(held).includes(String(narrowCount));
            JSON.stringify([lengthDifference, missing, item, hasIndex, indexed, enumerated])"#,
        )
        .unwrap(),
        r#"[1,"","--wide",false,"--wide",false]"#
    );
}

#[test]
fn realtime_cssom_native_container_units_share_current_style_and_geometry() {
    let mut vm = realtime_vm(
        r#"<!doctype html><style>
        #target { width: 10cqw; color: blue }
        #variable { --measure: calc(5cqi + 2px); font-size: var(--measure) }
        </style><div id=box style='container-type:inline-size;width:200px'>
        <div id=target style='font-size:10cqw'><div id=inherited></div></div><div id=variable></div></div>"#,
    );
    assert_eq!(
        vm.eval("const held = getComputedStyle(target); void box.offsetWidth; held.fontSize")
            .unwrap(),
        "20px"
    );
    let builds = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval("box.style.width = '450px'; held.fontSize").unwrap(),
        "45px"
    );
    assert_eq!(
        vm.eval("JSON.stringify([held.color, held.fontSize])")
            .unwrap(),
        r#"["rgb(0, 0, 255)","45px"]"#,
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
    assert_eq!(
        vm.eval(
            "box.style.width = '350px'; getComputedStyle(inherited).getPropertyValue('font-size')"
        )
        .unwrap(),
        "35px"
    );
    assert_eq!(
        vm.eval("box.style.width = '400px'; getComputedStyle(variable).fontSize")
            .unwrap(),
        "22px"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 3);
}

#[test]
fn realtime_cssom_font_relative_values_refresh_container_dependent_font_sizes() {
    let mut vm = realtime_vm(
        r#"<!doctype html><div id=box style='container-type:inline-size;width:200px'>
        <div style='font-size:10cqw'><div id=target style='font-size:2em; outline-offset:1em; color:blue'></div></div></div>"#,
    );
    assert_eq!(
        vm.eval("const held = getComputedStyle(target); void box.offsetWidth; held.fontSize")
            .unwrap(),
        "40px"
    );
    let builds = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval("box.style.width = '400px'; held.fontSize").unwrap(),
        "80px"
    );
    assert_eq!(
        vm.eval("box.style.width = '350px'; held.outlineOffset")
            .unwrap(),
        "70px"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 2);
}

#[test]
fn realtime_cssom_registered_container_lengths_refresh_direct_and_variable_reads() {
    let mut vm = realtime_vm(
        r#"<!doctype html><style>
        @property --size { syntax:'<length>'; inherits:false; initial-value:0px }
        #target { --size:10cqw; --raw:10cqw; --alias:var(--size); --label:blue;
                  font-size:var(--size); outline-offset:var(--alias); color:var(--label) }
        </style><div id=box style='container-type:inline-size;width:200px'><div id=target></div></div>"#,
    );
    assert_eq!(
        vm.eval("const held = getComputedStyle(target); void box.offsetWidth; JSON.stringify([held.getPropertyValue('--size'), held.fontSize])")
            .unwrap(),
        r#"["20px","20px"]"#
    );
    let builds = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval("box.style.width = '400px'; held.getPropertyValue('--size')")
            .unwrap(),
        "40px"
    );
    assert_eq!(
        vm.eval("JSON.stringify([held.getPropertyValue('--raw').trim(), held.color])")
            .unwrap(),
        r#"["10cqw","rgb(0, 0, 255)"]"#,
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
    assert_eq!(
        vm.eval("box.style.width = '450px'; held.fontSize").unwrap(),
        "45px"
    );
    assert_eq!(
        vm.eval("box.style.width = '500px'; held.outlineOffset")
            .unwrap(),
        "50px"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 3);
}

#[test]
fn on_demand_cssom_size_container_query_keeps_live_inline_sizes() {
    let mut vm = new_parsed_test_vm(
        "https://ondemand-cssom.test/",
        r#"<!doctype html><style>
        #target { color: red }
        @container (width >= 300px) { #target { color: green } }
        </style><div id=box style='container-type:inline-size;width:200px'>
        <div id=target></div></div>"#,
    );
    vm.set_layout_policy(LayoutPolicy::Lazy);
    assert_eq!(vm.eval("box.offsetWidth").unwrap(), "200");
    let builds = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval("box.style.width = '400px'; getComputedStyle(target).color")
            .unwrap(),
        "rgb(0, 128, 0)"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
}

#[test]
fn realtime_size_container_queries_converge_through_twelve_nested_containers() {
    const DEPTH: usize = 12;
    let mut markup = String::from(
        "<!doctype html><style>.container {container-type:inline-size;width:100px}\
         #c0 {width:400px} #target {width:20px}",
    );
    for child in 1..DEPTH {
        writeln!(
            markup,
            "@container c{} (width >= 300px) {{ #c{child} {{width:400px}} }}",
            child - 1,
        )
        .unwrap();
    }
    write!(
        markup,
        "@container c{} (width >= 300px) {{ #target {{width:110px}} }}</style>",
        DEPTH - 1,
    )
    .unwrap();
    for index in 0..DEPTH {
        write!(
            markup,
            "<div id=c{index} class=container style='container-name:c{index}'>",
        )
        .unwrap();
    }
    markup.push_str("<div id=target></div>");
    markup.push_str(&"</div>".repeat(DEPTH));

    let mut vm = realtime_vm(&markup);
    let builds = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval("JSON.stringify([target.offsetWidth, target.offsetWidth])")
            .unwrap(),
        "[110,110]"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
}

#[test]
fn realtime_size_container_query_uses_table_caption_content_box() {
    let mut vm = realtime_vm(
        r#"<!doctype html><style>
        #target { width: 20px }
        @container cap (width >= 300px) { #target { width: 110px } }
        </style><div style='container-type:inline-size;width:600px'>
        <table style='width:400px'>
        <caption style='container:cap / inline-size;width:400px'><div id=target></div></caption>
        </table></div>"#,
    );
    assert_eq!(vm.eval("target.offsetWidth").unwrap(), "110");
}

#[test]
fn realtime_cssom_shared_adopted_sheet_refreshes_all_shadow_clients_in_same_turn() {
    for first_read in ["color", "width", "offsetWidth"] {
        let mut vm =
            realtime_vm("<!doctype html><section id=left></section><section id=right></section>");
        vm.eval(&format!("globalThis.firstSharedRead={first_read:?}"))
            .unwrap();
        let initial = vm
            .eval(
                r#"(() => {
          const roots = ['left', 'right'].map(id =>
            document.getElementById(id).attachShadow({mode: 'open'}));
          const targets = roots.map(root => {
            const target = root.appendChild(document.createElement('div'));
            target.className = 'shared-target';
            return target;
          });
          const sheet = new CSSStyleSheet();
          sheet.replaceSync('.shared-target {display:block;width:100px;color:rgb(1,2,3)}');
          roots.forEach(root => { root.adoptedStyleSheets = [sheet]; });
          const held = targets.map(target => getComputedStyle(target));
          globalThis.sharedSheet = sheet;
          globalThis.readShared = () => {
            const first = firstSharedRead === 'color' ? held.map(style => style.color)
              : firstSharedRead === 'width' ? held.map(style => style.width)
              : targets.map(target => target.offsetWidth);
            return [first, held.map(style => style.color), held.map(style => style.width),
                    targets.map(target => target.offsetWidth), held.map(style => style.display)];
          };
          return JSON.stringify(readShared());
        })()"#,
            )
            .unwrap();
        let expected = |color: &str, width: u32, display: &str| {
            let px = format!("{width}px");
            let first = match first_read {
                "color" => serde_json::json!([color, color]),
                "width" => serde_json::json!([px, px]),
                "offsetWidth" => serde_json::json!([width, width]),
                _ => unreachable!(),
            };
            serde_json::json!([
                first,
                [color, color],
                [px, px],
                [width, width],
                [display, display]
            ])
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&initial).unwrap(),
            expected("rgb(1, 2, 3)", 100, "block"),
            "initial state, first read {first_read}"
        );
        let builds = vm.layout_pass_observability_for_test().1;
        // Capture the first answers immediately after each mutation, while the
        // same JS evaluation is still active and the original wrappers are held.
        let updated = vm
            .eval(
                r#"(() => {
          sharedSheet.replaceSync('.shared-target {display:block;width:200px;color:rgb(4,5,6)}');
          const replaced = readShared();
          const declaration = sharedSheet.cssRules[0].style;
          declaration.width = '300px';
          declaration.color = 'rgb(7,8,9)';
          const edited = readShared();
          const editedClean = readShared();
          declaration.display = 'flow-root';
          declaration.width = '400px';
          declaration.color = 'rgb(10,11,12)';
          return JSON.stringify([replaced, edited, editedClean, readShared(), readShared()]);
        })()"#,
            )
            .unwrap();
        let replacement = expected("rgb(4, 5, 6)", 200, "block");
        let edit = expected("rgb(7, 8, 9)", 300, "block");
        let flow_root = expected("rgb(10, 11, 12)", 400, "flow-root");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&updated).unwrap(),
            serde_json::json!([replacement, edit, edit, flow_root, flow_root]),
            "first read {first_read}"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, builds + 3);
    }
}

#[test]
fn realtime_cssom_pure_root_style_and_inline_reads_do_not_build_layout() {
    let mut vm =
        realtime_vm("<!doctype html><div id=target style='width:100px;color:rgb(1,2,3)'></div>");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "const style=getComputedStyle(target);target.style.width='200px';\
             JSON.stringify([style.color,style.getPropertyValue('color'),target.style.width])",
        )
        .unwrap(),
        r#"["rgb(1, 2, 3)","rgb(1, 2, 3)","200px"]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    assert_eq!(vm.eval("getComputedStyle(target).width").unwrap(), "200px");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_cssom_child_color_observes_current_parent_viewport() {
    let mut vm = realtime_vm(
        "<!doctype html><iframe id=frame style='width:200px;height:100px;border:0'></iframe>",
    );
    vm.eval(
        "window.child=frame.contentWindow;child.document.open();\
         child.document.write('<!doctype html><style>#target{color:rgb(1,2,3)}@media(min-width:250px){#target{color:rgb(4,5,6)}}</style><div id=target></div>');\
         child.document.close();window.style=child.getComputedStyle(child.document.getElementById('target'))",
    )
    .unwrap();
    assert_eq!(vm.eval("style.color").unwrap(), "rgb(1, 2, 3)");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval("frame.style.width='320px';style.getPropertyValue('color')")
            .unwrap(),
        "rgb(4, 5, 6)"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    assert_eq!(vm.eval("style.color").unwrap(), "rgb(4, 5, 6)");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_cssom_grid_tracks_share_current_geometry() {
    let mut vm = realtime_vm(
        "<!doctype html><div id=target style='display:grid;width:200px;grid-template-columns:1fr 1fr'><div>A</div><div>B</div></div>",
    );
    assert_eq!(
        vm.eval("getComputedStyle(target).gridTemplateColumns")
            .unwrap(),
        "100px 100px"
    );
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "target.style.width='400px';JSON.stringify([getComputedStyle(target).getPropertyValue('grid-template-columns'),target.offsetWidth])",
        )
        .unwrap(),
        r#"["200px 200px",400]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[cfg(feature = "devtools")]
#[test]
fn realtime_cssom_protocol_batch_refreshes_once_before_observation() {
    let mut vm = realtime_vm(
        "<!doctype html><div id=container style='width:200px'>\
         <div id=first style='width:50%;color:rgb(1,2,3)'></div>\
         <div id=second style='width:25%;color:rgb(1,2,3)'></div></div>",
    );
    let handles = {
        let host = vm._context_host.borrow();
        ["first", "second"].map(|id| host.dom_host().element_handle_by_id(id).unwrap())
    };
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.computed_style_property_values_for_document_snapshot(handles, &["color".into()])
            .unwrap(),
        vec![vec!["rgb(1, 2, 3)"], vec!["rgb(1, 2, 3)"]]
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    let properties = ["width", "inline-size"].map(String::from);
    assert_eq!(
        vm.computed_style_property_values_for_document_snapshot(handles, &properties)
            .unwrap(),
        vec![vec!["100px", "100px"], vec!["50px", "50px"]]
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    vm.eval("container.style.width='400px'").unwrap();
    for _ in 0..2 {
        assert_eq!(
            vm.computed_style_property_values_for_document_snapshot(handles, &properties)
                .unwrap(),
            vec![vec!["200px", "200px"], vec!["100px", "100px"]]
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
    }
}

#[cfg(feature = "devtools")]
#[test]
fn realtime_cssom_inspector_enumeration_refreshes_used_values() {
    let mut vm = realtime_vm("<!doctype html><div id=target style='width:100px'></div>");
    let target = vm
        ._context_host
        .borrow()
        .dom_host()
        .element_handle_by_id("target")
        .unwrap();
    let before = vm.layout_pass_observability_for_test().1;
    for (width, builds, change_width) in [(100, 1, false), (240, 2, true), (240, 2, false)] {
        if change_width {
            vm.eval("target.style.width='240px'").unwrap();
        }
        let properties = vm
            .computed_style_properties_for_inspector_handle(target)
            .unwrap()
            .unwrap()
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(properties["width"], format!("{width}px"));
        assert_eq!(vm.layout_pass_observability_for_test().1, before + builds);
    }
}

const CONTAINER_DEPENDENT_DURATION: &str = "calc(10s + (sign(2cqw - 10px) * 5s))";

fn container_duration_vm(
    animation_duration: &str,
    transition_duration: &str,
) -> StandaloneScriptVmHarness {
    let markup = format!(
        r#"<!doctype html><style>
        #container {{ container-type:inline-size; width:100px; height:20px }}
        </style><div id=container>
        <div id=target style="animation-duration:{animation_duration};transition-duration:{transition_duration};color:rgb(1,2,3)"></div>
        <div id=second style="animation-duration:{animation_duration};transition-duration:{transition_duration};color:rgb(1,2,3)"></div>
        </div>"#,
    );
    let mut vm = realtime_vm(&markup);
    // Establish the previous container geometry explicitly. Every tested
    // write/read below must refresh through CSSOM, without a geometry getter.
    assert_eq!(
        vm.eval(
            "window.containerRule=document.styleSheets[0].cssRules[0];\
             window.heldDurationStyle=getComputedStyle(document.getElementById('target'));\
             document.getElementById('container').offsetWidth",
        )
        .unwrap(),
        "100"
    );
    vm
}

#[test]
fn realtime_cssom_retained_duration_declaration_refreshes_container_dependencies() {
    for (js_property, css_property) in [
        ("animationDuration", "animation-duration"),
        ("transitionDuration", "transition-duration"),
    ] {
        for method_first in [false, true] {
            let mut vm =
                container_duration_vm(CONTAINER_DEPENDENT_DURATION, CONTAINER_DEPENDENT_DURATION);
            let getter = format!("heldDurationStyle.{js_property}");
            let method = format!("heldDurationStyle.getPropertyValue('{css_property}')");
            let (first, second) = if method_first {
                (&method, &getter)
            } else {
                (&getter, &method)
            };
            assert_eq!(vm.eval(first).unwrap(), "5s");
            let before = vm.layout_pass_observability_for_test().1;
            let reads = format!("JSON.stringify([{first},{second}])");
            assert_eq!(
                vm.eval(&format!("containerRule.style.width='1000px';{reads}"))
                    .expect("public duration read must refresh without reentering layout"),
                r#"["15s","15s"]"#,
                "{css_property}, method_first={method_first}"
            );
            assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
            assert_eq!(vm.eval(&reads).unwrap(), r#"["15s","15s"]"#);
            assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
        }
    }
}

#[test]
fn realtime_cssom_duration_shorthands_refresh_before_serializing_longhands() {
    for property in ["animation", "transition"] {
        let mut vm =
            container_duration_vm(CONTAINER_DEPENDENT_DURATION, CONTAINER_DEPENDENT_DURATION);
        assert_eq!(
            vm.eval(&format!("heldDurationStyle.{property}")).unwrap(),
            "5s"
        );
        let before = vm.layout_pass_observability_for_test().1;
        assert_eq!(
            vm.eval(&format!(
                "containerRule.style.width='1000px';heldDurationStyle.{property}"
            ))
            .unwrap(),
            "15s"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
        // getPropertyValue must also work as the first read after a different
        // mutation; the preceding getter cannot make this assertion pass.
        assert_eq!(
            vm.eval(&format!(
                "containerRule.style.width='100px';heldDurationStyle.getPropertyValue('{property}')"
            ))
            .unwrap(),
            "5s"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
        assert_eq!(
            vm.eval(&format!(
                "JSON.stringify([heldDurationStyle.{property},heldDurationStyle.getPropertyValue('{property}')])"
            ))
            .unwrap(),
            r#"["5s","5s"]"#
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
    }
}

#[cfg(feature = "devtools")]
#[test]
fn realtime_cssom_duration_protocol_batch_refreshes_once_before_style_observation() {
    let mut vm = container_duration_vm(CONTAINER_DEPENDENT_DURATION, CONTAINER_DEPENDENT_DURATION);
    let handles = {
        let host = vm._context_host.borrow();
        ["target", "second"].map(|id| host.dom_host().element_handle_by_id(id).unwrap())
    };
    // No used-size property is requested: these durations alone must establish
    // current container geometry before the batch owns its style observation.
    let properties = ["animation-duration", "transition-duration"].map(String::from);
    assert_eq!(
        vm.computed_style_property_values_for_document_snapshot(handles, &properties)
            .unwrap(),
        vec![vec!["5s", "5s"], vec!["5s", "5s"]]
    );
    let before = vm.layout_pass_observability_for_test().1;
    vm.eval("containerRule.style.width='1000px'").unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    for _ in 0..2 {
        assert_eq!(
            vm.computed_style_property_values_for_document_snapshot(handles, &properties)
                .expect("batch freshness must finish before its style borrow"),
            vec![vec!["15s", "15s"], vec!["15s", "15s"]]
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    }
}

#[test]
fn realtime_cssom_reads_in_size_container_share_one_fresh_layout() {
    let mut vm = container_duration_vm("2s", "3s");
    let before = vm.layout_pass_observability_for_test().1;
    let reads = "JSON.stringify([heldDurationStyle.animationDuration,\
        heldDurationStyle.getPropertyValue('animation-duration'),\
        heldDurationStyle.transitionDuration,\
        heldDurationStyle.getPropertyValue('transition-duration'),\
        heldDurationStyle.color,heldDurationStyle.getPropertyValue('color')])";
    assert_eq!(
        vm.eval(&format!("containerRule.style.width='1000px';{reads}"))
            .unwrap(),
        r#"["2s","2s","3s","3s","rgb(1, 2, 3)","rgb(1, 2, 3)"]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    #[cfg(feature = "devtools")]
    {
        let handles = {
            let host = vm._context_host.borrow();
            ["target", "second"].map(|id| host.dom_host().element_handle_by_id(id).unwrap())
        };
        let properties = ["animation-duration", "transition-duration", "color"].map(String::from);
        for _ in 0..2 {
            assert_eq!(
                vm.computed_style_property_values_for_document_snapshot(handles, &properties)
                    .unwrap(),
                vec![
                    vec!["2s", "3s", "rgb(1, 2, 3)"],
                    vec!["2s", "3s", "rgb(1, 2, 3)"]
                ]
            );
            assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
        }
    }
    assert_eq!(
        vm.eval(reads).unwrap(),
        r#"["2s","2s","3s","3s","rgb(1, 2, 3)","rgb(1, 2, 3)"]"#
    );
    assert_eq!(vm.eval("container.offsetWidth").unwrap(), "1000");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_cssom_container_numeric_vars_keep_values_across_layout_refreshes() {
    // Inline container sizes exercise the engine's existing Stylo query_size
    // input for var() substitution. Rule-defined used sizes are independently
    // covered by the direct supplemental-value regression above.
    for invalid_definition in ["", "--measure:var(--measure);"] {
        let expression = "var(--measure, calc(10s + (sign(2cqw - 10px) * 5s)))";
        let mut vm = realtime_vm(&format!(
            r#"<!doctype html><div id=container style="container-type:inline-size;width:100px;height:20px">
                <div id=target style="{invalid_definition}animation-duration:{expression};transition-duration:{expression}"></div>
            </div>"#,
        ));
        assert_eq!(
            vm.eval("window.variableStyle=getComputedStyle(target);container.offsetWidth")
                .unwrap(),
            "100"
        );
        assert_eq!(vm.eval(
            "JSON.stringify([variableStyle.getPropertyValue('--measure'),variableStyle.animationDuration,variableStyle.transitionDuration])"
        ).unwrap(), r#"["","5s","5s"]"#);
        let before = vm.layout_pass_observability_for_test().1;
        assert_eq!(vm.eval(
            "container.style.width='1000px';JSON.stringify([variableStyle.animationDuration,variableStyle.getPropertyValue('transition-duration')])"
        ).unwrap(), r#"["15s","15s"]"#);
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
        assert_eq!(
            vm.eval(
                "JSON.stringify([variableStyle.animationDuration,variableStyle.transitionDuration])"
            )
            .unwrap(),
            r#"["15s","15s"]"#
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    }

    let mut vm = realtime_vm(
        r#"<!doctype html><div id=container style="container-type:inline-size;width:100px;height:20px;--measure:2s">
            <div id=target style="animation-duration:var(--measure,calc(10s + (sign(2cqw - 10px) * 5s)));transition-duration:var(--measure,calc(10s + (sign(2cqw - 10px) * 5s)))"></div>
        </div>"#,
    );
    assert_eq!(
        vm.eval("window.variableStyle=getComputedStyle(target);container.offsetWidth")
            .unwrap(),
        "100"
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify([variableStyle.animationDuration,variableStyle.transitionDuration])"
        )
        .unwrap(),
        r#"["2s","2s"]"#
    );
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(vm.eval(
        "container.style.width='1000px';JSON.stringify([variableStyle.animationDuration,variableStyle.getPropertyValue('transition-duration')])"
    ).unwrap(), r#"["2s","2s"]"#);
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    assert_eq!(
        vm.eval(
            "JSON.stringify([variableStyle.animationDuration,variableStyle.transitionDuration])"
        )
        .unwrap(),
        r#"["2s","2s"]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_cssom_inherited_zoom_refreshes_the_source_container_dependency() {
    // The existing computed_style_resolves_zoom regression already supports
    // zoom:inherit and this numeric cq expression. Keep the source as a child
    // of the size container, so its own query never depends on itself.
    let mut vm = realtime_vm(
        r#"<!doctype html><style>#container {container-type:inline-size;width:100px;height:20px}</style>
        <div id=container><div id=zoomSource style="zoom:calc(100% + (sign(2cqw - 10px) * 50%))">
        <div id=target style="zoom:inherit"></div></div></div>"#,
    );
    assert_eq!(vm.eval(
        "window.zoomRule=document.styleSheets[0].cssRules[0];window.zoomStyle=getComputedStyle(target);container.offsetWidth"
    ).unwrap(), "100");
    assert_eq!(vm.eval("zoomStyle.zoom").unwrap(), "0.5");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(vm.eval(
        "zoomRule.style.width='1000px';JSON.stringify([zoomStyle.zoom,zoomStyle.getPropertyValue('zoom')])"
    ).unwrap(), r#"["1.5","1.5"]"#);
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    assert_eq!(vm.eval("zoomStyle.zoom").unwrap(), "1.5");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_cssom_existing_custom_function_projection_refreshes_container_width() {
    // Reuse the existing no-argument renderer @function projection; this does
    // not assert new stylesheet container-query or arbitrary function support.
    let mut vm = realtime_vm(
        r#"<!doctype html><style>
        #container {container:--measure / size;width:100px;height:20px}
        @function --measured() {
            @container --measure (width = 100px) {result:100px;}
            @container --measure (width = 1000px) {result:1000px;}
        }
        #target {--actual:--measured();--literal:1cqw}
        </style><div id=container><div id=target></div></div>"#,
    );
    assert_eq!(vm.eval(
        "window.functionRule=document.styleSheets[0].cssRules[0];window.functionStyle=getComputedStyle(target);container.offsetWidth"
    ).unwrap(), "100");
    assert_eq!(
        vm.eval("functionStyle.getPropertyValue('--actual')")
            .unwrap(),
        "100px"
    );
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval("functionRule.style.width='1000px';functionStyle.getPropertyValue('--actual')")
            .unwrap(),
        "1000px"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    assert_eq!(
        vm.eval("functionStyle.getPropertyValue('--literal')")
            .unwrap(),
        "1cqw"
    );
    assert_eq!(
        vm.eval("functionStyle.getPropertyValue('--actual')")
            .unwrap(),
        "1000px"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn legacy_css_clip_hides_paint_hit_targets_without_changing_box_geometry() {
    let mut vm = realtime_vm(
        r#"<!doctype html><style>
        body { margin: 0 }
        #target { position: absolute; left: 10px; top: 10px; width: 60px; height: 40px;
                  clip: rect(1px, 1px, 1px, 1px) }
        #target:focus { clip: rect(auto, auto, auto, 20px) }
        #fixed { position: fixed; left: 30px; top: 20px; width: 10px; height: 10px }
        </style><div id=target tabindex=0><div id=fixed></div></div>"#,
    );
    {
        use moli_layout::{
            LayoutFlushReason, LayoutPassRequest, LayoutRect, LayoutViewport, PaintFragment,
            PaintShape,
        };
        let runtime = vm._context_host.borrow();
        let pass = runtime
            .build_layout_pass_for_document(
                runtime.document_handle(),
                LayoutPassRequest::with_paint(
                    LayoutViewport::new(120, 90, 1.0),
                    LayoutFlushReason::Test,
                ),
            )
            .unwrap()
            .unwrap();
        assert!(
            pass.paint_snapshot()
                .unwrap()
                .fragments
                .iter()
                .any(|fragment| {
                    matches!(fragment, PaintFragment::PushClip { shape: PaintShape::Rect(rect), .. }
                if *rect == LayoutRect::new(1.0, 1.0, 0.0, 0.0))
                })
        );
    }
    assert_eq!(
        vm.eval(
            r#"const sample = () => [15, 35, 50].map(x => document.elementFromPoint(x, 25)?.id ?? '');
            const clipped = sample();
            target.focus();
            JSON.stringify([target.getBoundingClientRect().width, clipped, sample()])"#,
        )
        .unwrap(),
        r#"[60,["","",""],["","fixed","target"]]"#
    );
}

#[test]
fn legacy_css_clip_applies_to_filter_output() {
    // Exercise the filter order from WPT css/css-masking/clip/clip-filter-order.html,
    // and a shadow whose only source lies outside the CSS clip.
    let mut vm = realtime_vm(
        r#"<!doctype html><style>
        html, body { margin: 0; background: white }
        .clipped { position: absolute; width: 80px; height: 55px;
                   clip: rect(0, 40px, 55px, 0) }
        #blur { left: 20px; top: 20px; background: red; filter: blur(4px) }
        #shadow { left: 20px; top: 100px; filter: drop-shadow(-40px 0 green) }
        #shadow-source { margin-left: 40px; width: 40px; height: 55px; background: red }
        #ancestor { left: 160px; top: 20px }
        #filtered-child { width: 80px; height: 55px; background: green;
                          filter: drop-shadow(12px 0 4px red) }
        </style>
        <div id=blur class=clipped></div>
        <div id=shadow class=clipped><div id=shadow-source></div></div>
        <div id=ancestor class=clipped><div id=filtered-child></div></div>"#,
    );
    let snapshot = vm
        .screenshot_layout_snapshot(moli_layout::PaintViewport::new(260, 175, 1.0))
        .unwrap()
        .unwrap();
    let image = moli_paint::raster_snapshot(&snapshot).unwrap();
    let pixel = |x, y| {
        let index = ((y * image.width + x) * 4) as usize;
        &image.rgba[index..index + 4]
    };
    assert_eq!(pixel(58, 45), [255, 0, 0, 255]);
    assert_eq!(pixel(62, 45), [255, 255, 255, 255]);
    assert_eq!(pixel(45, 125), [0, 128, 0, 255]);
    assert_eq!(pixel(62, 125), [255, 255, 255, 255]);
    assert_eq!(pixel(198, 45), [0, 128, 0, 255]);
    assert_eq!(pixel(202, 45), [255, 255, 255, 255]);
}
