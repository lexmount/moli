use super::*;
use crate::runtime::{RendererDragData, RendererDragDataItem, RendererPointerEventProperties};

fn payload_vm(source: &str) -> StandaloneScriptVmHarness {
    new_rendered_test_vm(
        "https://native-drag-payload.test/pages/source.html",
        &format!(
            r#"<!doctype html><html><head><base href="https://assets.drag.test/documents/"></head>
<body style="margin:0"><style>
#source {{position:absolute;left:0;top:0;width:300px;height:60px;font:20px/24px monospace}}
#target {{position:absolute;left:0;top:120px;width:400px;height:100px}}
</style>{source}<div id=target contenteditable=true></div></body></html>"#
        ),
    )
}

fn intercepted_payload(vm: &mut StandaloneScriptVmHarness, point: [f64; 2]) -> RendererDragData {
    vm.publish_layout_for_test().unwrap();
    for (event, x) in [("mousedown", point[0]), ("mousemove", point[0] + 12.0)] {
        let outcome = vm
            .dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                x,
                point[1],
                event,
                if event == "mousedown" { 0 } else { -1 },
                Some(1),
                0,
                0.0,
                0.0,
                RendererPointerEventProperties::default(),
                0,
                true,
            )
            .unwrap();
        if event == "mousedown"
            && let Some(gesture) = vm.pending_drag_gesture.as_mut()
        {
            // These payload tests exercise a deliberate text drag, after
            // macOS's selection-adjustment interval.
            gesture.pressed_at -= std::time::Duration::from_millis(250);
        }
        if event == "mousemove" {
            let intercepted = outcome.intercepted_drag.expect("native default payload");
            return RendererDragData {
                items: intercepted.items,
                files: Vec::new(),
                directories: Vec::new(),
                drag_operations_mask: intercepted.drag_operations_mask,
            };
        }
    }
    unreachable!()
}

#[test]
fn native_selection_drag_distinguishes_quick_and_held_gestures_on_macos() {
    for held in [false, true] {
        let mut vm = payload_vm("<div id=source>selected text</div>");
        let point: [f64; 2] = serde_json::from_str(&vm.eval(r#"(() => {
          const range=document.createRange();range.selectNodeContents(document.getElementById('source'));
          getSelection().removeAllRanges();getSelection().addRange(range);
          const rect=range.getClientRects()[0];return JSON.stringify([rect.left+2,rect.top+2]);
        })()"#).unwrap()).unwrap();
        vm.publish_layout_for_test().unwrap();
        vm.dispatch_mouse_event_at_point(point[0], point[1], "mousedown", 0, Some(1), 0.0, 0.0)
            .unwrap();
        if held {
            vm.pending_drag_gesture.as_mut().unwrap().pressed_at -=
                std::time::Duration::from_millis(250);
        }
        let result = vm
            .dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                point[0] + 12.0,
                point[1],
                "mousemove",
                -1,
                Some(1),
                0,
                0.0,
                0.0,
                RendererPointerEventProperties::default(),
                0,
                true,
            )
            .unwrap();
        assert_eq!(
            result.intercepted_drag.is_some(),
            held || !cfg!(target_os = "macos")
        );
        // Crossing the threshold consumes even an ineligible quick gesture;
        // waiting later while still holding the button must not restart it.
        assert!(vm.pending_drag_gesture.is_none());
    }
}

fn item<'a>(data: &'a RendererDragData, mime: &str) -> &'a RendererDragDataItem {
    data.items
        .iter()
        .find(|item| item.mime_type == mime)
        .unwrap()
}

fn drop_into_editor(vm: &mut StandaloneScriptVmHarness, data: RendererDragData) {
    vm.eval(
        r#"(() => {
          const target=document.getElementById('target');
          window.__dropInputTypes=[];
          target.addEventListener('input',event=>window.__dropInputTypes.push(event.inputType));
        })()"#,
    )
    .unwrap();
    for event in ["dragenter", "drop"] {
        vm.dispatch_drag_event_at_point(20.0, 140.0, event, data.clone(), 0)
            .expect("default rich-text drop");
    }
    assert_eq!(
        vm.eval("JSON.stringify(window.__dropInputTypes)").unwrap(),
        r#"["insertFromDrop"]"#
    );
}

#[test]
fn selected_native_drag_preserves_partial_rich_markup_and_document_base_metadata() {
    let mut vm = payload_vm("<div id=source><b id=bold>😀AB</b><em id=italic>CD</em></div>");
    let point: [f64; 2] = serde_json::from_str(
        &vm.eval(
            r#"(() => {
              const range=document.createRange();
              range.setStart(document.getElementById('bold').firstChild,2);
              range.setEnd(document.getElementById('italic').firstChild,1);
              const selection=getSelection();selection.removeAllRanges();selection.addRange(range);
              const rect=range.getClientRects()[0];
              window.__nativePayloadCalls=0;
              window.__savedRange=window.Range;
              window.__savedClone=Range.prototype.cloneContents;
              window.__savedRangeString=Range.prototype.toString;
              window.__savedSelectionString=Selection.prototype.toString;
              const fail=()=>{window.__nativePayloadCalls++;throw new Error('author payload callback');};
              Range.prototype.cloneContents=Range.prototype.toString=Selection.prototype.toString=fail;
              Object.defineProperty(window,'Range',{configurable:true,get:fail});
              return JSON.stringify([rect.left+2,rect.top+2]);
            })()"#,
        )
        .unwrap(),
    )
    .unwrap();
    let data = intercepted_payload(&mut vm, point);
    assert_eq!(item(&data, "text/plain").data, "ABC");
    assert_eq!(
        item(&data, "text/html").data,
        "<b id=\"bold\">AB</b><em id=\"italic\">C</em>"
    );
    assert_eq!(
        item(&data, "text/html").base_url.as_deref(),
        Some("https://native-drag-payload.test/pages/source.html")
    );
    assert_eq!(vm.eval("String(window.__nativePayloadCalls)").unwrap(), "0");
    vm.eval(
        r#"Object.defineProperty(window,'Range',{configurable:true,writable:true,value:window.__savedRange});
Range.prototype.cloneContents=window.__savedClone;
Range.prototype.toString=window.__savedRangeString;
Selection.prototype.toString=window.__savedSelectionString;"#,
    )
    .unwrap();
    drop_into_editor(&mut vm, data);
    assert_eq!(
        vm.eval("JSON.stringify([document.querySelector('#target b').textContent,document.querySelector('#target em').textContent,document.getElementById('target').textContent])").unwrap(),
        r#"["AB","C","ABC"]"#
    );
}

#[test]
fn native_selection_drag_keeps_inline_container_and_omits_hidden_descendants() {
    for (source, expected_text, expected_html) in [
        ("<b id=source>Rich</b>", "Rich", "<b id=\"source\">Rich</b>"),
        (
            "<div id=source><b>A</b><span style='display:none'>hidden</span><em>B</em></div>",
            "AB",
            "<b>A</b><em>B</em>",
        ),
    ] {
        let mut vm = payload_vm(source);
        let point: [f64; 2] = serde_json::from_str(
            &vm.eval(
                r#"(() => {
                  const range=document.createRange();range.selectNodeContents(document.getElementById('source'));
                  const selection=getSelection();selection.removeAllRanges();selection.addRange(range);
                  const rect=range.getClientRects()[0];
                  return JSON.stringify([rect.left+2,rect.top+2]);
                })()"#,
            )
            .unwrap(),
        )
        .unwrap();
        let data = intercepted_payload(&mut vm, point);
        assert_eq!(item(&data, "text/plain").data, expected_text);
        assert_eq!(item(&data, "text/html").data, expected_html);
        drop_into_editor(&mut vm, data);
        assert_eq!(
            vm.eval("document.getElementById('target').innerHTML")
                .unwrap(),
            expected_html
        );
        assert_eq!(
            vm.eval("document.getElementById('target').textContent")
                .unwrap(),
            expected_text
        );
    }
}

#[test]
fn native_rich_drop_appends_without_target_focus_or_selection_and_keeps_existing_content() {
    for existing_content in [false, true] {
        for clear_selection in [false, true] {
            let mut vm = payload_vm("<div id=source><b>Rich</b><em> text</em></div>");
            let point: [f64; 2] = serde_json::from_str(
                &vm.eval(&format!(
                    r#"(() => {{
                      const target=document.getElementById('target');
                      if({existing_content})target.innerHTML='<strong>before</strong>';
                      const range=document.createRange();range.selectNodeContents(document.getElementById('source'));
                      const selection=getSelection();selection.removeAllRanges();selection.addRange(range);
                      const rect=range.getClientRects()[0];
                      return JSON.stringify([rect.left+2,rect.top+2]);
                    }})()"#
                ))
                .unwrap(),
            )
            .unwrap();
            let data = intercepted_payload(&mut vm, point);
            if clear_selection {
                vm.eval("getSelection().removeAllRanges()").unwrap();
                assert_eq!(vm.eval("String(getSelection().rangeCount)").unwrap(), "0");
            } else {
                assert_eq!(
                    vm.eval("String(document.getElementById('source').contains(getSelection().anchorNode))").unwrap(),
                    "true"
                );
            }
            assert_eq!(
                vm.eval("String(document.activeElement===document.getElementById('target'))")
                    .unwrap(),
                "false"
            );
            drop_into_editor(&mut vm, data);
            assert_eq!(
                vm.eval("document.getElementById('target').innerHTML")
                    .unwrap(),
                if existing_content {
                    "<strong>before</strong><b>Rich</b><em> text</em>"
                } else {
                    "<b>Rich</b><em> text</em>"
                },
                "existing_content={existing_content}, clear_selection={clear_selection}"
            );
            assert_eq!(
                vm.eval("document.getElementById('source').innerHTML")
                    .unwrap(),
                "<b>Rich</b><em> text</em>",
                "appending a copy must not delete the source selection"
            );
            assert_eq!(
                vm.eval("JSON.stringify([getSelection().focusNode===document.getElementById('target'),getSelection().focusOffset,getSelection().isCollapsed])").unwrap(),
                if existing_content { "[true,3,true]" } else { "[true,2,true]" }
            );
        }
    }
}

#[test]
fn native_rich_drop_preserves_a_selection_inside_the_target_host() {
    let mut vm = payload_vm("<div id=source><b>Rich</b><em> text</em></div>");
    let point: [f64; 2] = serde_json::from_str(
        &vm.eval(
            r#"(() => {
              document.getElementById('target').innerHTML='<strong>before</strong>';
              const range=document.createRange();range.selectNodeContents(document.getElementById('source'));
              const selection=getSelection();selection.removeAllRanges();selection.addRange(range);
              const rect=range.getClientRects()[0];
              return JSON.stringify([rect.left+2,rect.top+2]);
            })()"#,
        )
        .unwrap(),
    )
    .unwrap();
    let data = intercepted_payload(&mut vm, point);
    vm.eval(
        r#"(() => {
          const range=document.createRange();
          const text=document.querySelector('#target strong').firstChild;
          range.setStart(text,1);range.setEnd(text,3);
          const selection=getSelection();selection.removeAllRanges();selection.addRange(range);
        })()"#,
    )
    .unwrap();
    drop_into_editor(&mut vm, data);
    assert_eq!(
        vm.eval("document.getElementById('target').innerHTML")
            .unwrap(),
        "<strong>b<b>Rich</b><em> text</em>ore</strong>"
    );
}

#[test]
fn native_link_drag_exports_resolved_url_title_and_rich_default_drop_without_author_getters() {
    let mut vm = payload_vm("<a id=source href='../details?q=1&amp;k=2'> Link <b>label</b> </a>");
    vm.eval(
        r#"(() => {
          const source=document.getElementById('source');window.__nativePayloadCalls=0;
          const fail=()=>{window.__nativePayloadCalls++;throw new Error('author DOM getter');};
          for(const property of ['href','textContent','outerHTML'])
            Object.defineProperty(source,property,{configurable:true,get:fail});
          source.getAttribute=fail;
        })()"#,
    )
    .unwrap();
    let data = intercepted_payload(&mut vm, [2.0, 2.0]);
    let url = "https://assets.drag.test/details?q=1&k=2";
    assert_eq!(item(&data, "text/uri-list").data, url);
    assert_eq!(
        item(&data, "text/uri-list").title.as_deref(),
        Some("Link label")
    );
    assert_eq!(item(&data, "text/plain").data, url);
    assert_eq!(item(&data, "text/html").base_url.as_deref(), Some(url));
    assert!(
        item(&data, "text/html")
            .data
            .contains("href=\"https://assets.drag.test/details?q=1&amp;k=2\"")
    );
    assert_eq!(vm.eval("String(window.__nativePayloadCalls)").unwrap(), "0");
    drop_into_editor(&mut vm, data);
    assert_eq!(
        vm.eval("JSON.stringify([document.querySelector('#target a').href,document.querySelector('#target b').textContent])").unwrap(),
        r#"["https://assets.drag.test/details?q=1&k=2","label"]"#
    );
}

#[test]
fn native_image_drag_uses_the_link_url_and_retains_image_markup_without_fabricated_files() {
    for linked in [false, true] {
        let image = "<img id=source src='../image.png' alt='Image title'>";
        let source = if linked {
            format!("<a href='../details'>{image}</a>")
        } else {
            image.to_owned()
        };
        let mut vm = payload_vm(&source);
        let data = intercepted_payload(&mut vm, [2.0, 2.0]);
        assert_eq!(
            item(&data, "text/uri-list").data,
            if linked {
                "https://assets.drag.test/details"
            } else {
                "https://assets.drag.test/image.png"
            }
        );
        assert_eq!(
            item(&data, "text/uri-list").title.as_deref(),
            Some("Image title")
        );
        assert_eq!(
            data.items
                .iter()
                .filter(|item| item.mime_type == "text/plain")
                .count(),
            0
        );
        let html = &item(&data, "text/html").data;
        assert!(html.starts_with("<img "), "{html}");
        assert!(
            html.contains("src=\"https://assets.drag.test/image.png\""),
            "{html}"
        );
        assert!(
            !html.contains("<a"),
            "only the image supplies the image drag HTML"
        );
        assert!(data.files.is_empty());
        drop_into_editor(&mut vm, data);
        assert_eq!(
            vm.eval("document.querySelector('#target img').getAttribute('src')")
                .unwrap(),
            "https://assets.drag.test/image.png"
        );
    }
}

#[test]
fn responsive_native_image_drag_uses_selected_url_but_preserves_image_markup() {
    for (source, selected_name, html_src, html_srcset) in [
        (
            "<img id=source srcset='../only.png 1x' alt='Responsive image'>",
            "only.png",
            None,
            Some("../only.png 1x"),
        ),
        (
            "<img id=source src='../fallback.png' srcset='../selected.png 1x' alt='Responsive image'>",
            "selected.png",
            Some("https://assets.drag.test/fallback.png"),
            Some("../selected.png 1x"),
        ),
        (
            "<picture><source srcset='../picture.png 1x'><img id=source src='../fallback.png' alt='Responsive image'></picture>",
            "picture.png",
            Some("https://assets.drag.test/fallback.png"),
            None,
        ),
    ] {
        for linked in [false, true] {
            let source = if linked {
                format!("<a href='../details'>{source}</a>")
            } else {
                source.to_owned()
            };
            let mut vm = payload_vm(&source);
            let data = intercepted_payload(&mut vm, [2.0, 2.0]);
            let selected_url = format!("https://assets.drag.test/{selected_name}");
            assert_eq!(
                item(&data, "text/uri-list").data,
                if linked {
                    "https://assets.drag.test/details"
                } else {
                    &selected_url
                },
                "linked={linked}, source={source}"
            );
            assert_eq!(
                item(&data, "text/uri-list").title.as_deref(),
                Some("Responsive image")
            );
            assert!(data.items.iter().all(|item| item.mime_type != "text/plain"));
            let html = item(&data, "text/html");
            assert_eq!(html.base_url.as_deref(), Some("about:blank#blocked"));
            assert!(html.data.starts_with("<img "), "{}", html.data);
            assert!(!html.data.contains("<picture"), "{}", html.data);
            assert!(!html.data.contains("<source"), "{}", html.data);
            assert!(!html.data.contains("<a"), "{}", html.data);
            // Chromium exports the selected resource as the URI, while its
            // HTML serializer retains img attributes, including srcset and
            // the fallback src. It does not export a containing picture.
            if let Some(src) = html_src {
                assert!(
                    html.data.contains(&format!("src=\"{src}\"")),
                    "{}",
                    html.data
                );
            } else {
                assert!(!html.data.contains("src="), "{}", html.data);
            }
            if let Some(srcset) = html_srcset {
                assert!(
                    html.data.contains(&format!("srcset=\"{srcset}\"")),
                    "{}",
                    html.data
                );
            } else {
                assert!(!html.data.contains("srcset="), "{}", html.data);
            }
        }
    }
}
