use super::*;
use crate::runtime::{
    RendererDevToolsPageCommand, RendererPageCommand, RendererPointerEventProperties,
};
use moli_page_types::LayoutPolicy;

fn editor_page(loader: &crate::network::ResourceRequestClient) -> PageVm {
    let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
        loader,
        Url::parse("https://example.com/text-control-interaction").unwrap(),
    );
    page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
    page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
        inner_width: 480,
        inner_height: 250,
        device_pixel_ratio: 1.0,
        ..Default::default()
    }))
    .unwrap();
    page.vm_mut()
        .eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
input,textarea{display:block;margin:0;position:absolute;left:16px;top:16px;
  width:260px;height:24px;box-sizing:border-box;border:0;padding:0;
  font-family:monospace;font-size:20px;line-height:24px;resize:none}
textarea{top:64px;height:100px}
#secret{top:180px}
</style>`;
document.body.innerHTML = '<input id="field" value="alpha beta gamma"><textarea id="area">short\n\nlonger words</textarea><input id="secret" type="password" value="A😀éZ">';
"#,
        )
        .unwrap();
    page.vm_mut().sync_live_document_style_sources();
    page
}

fn mouse(page: &mut PageVm, event: &str, point: (f64, f64), count: i32, modifiers: u8) {
    page.dispatch_renderer_page_command(RendererPageCommand::DevTools(
        RendererDevToolsPageCommand::DispatchMouseEventAtPoint {
            x: point.0,
            y: point.1,
            event_name: event.to_owned(),
            button: if event == "mousemove" { -1 } else { 0 },
            buttons: Some(i32::from(event != "mouseup")),
            click_count: count,
            delta_x: 0.0,
            delta_y: 0.0,
            pointer: RendererPointerEventProperties::default(),
            modifiers,
            intercept_drags: false,
        },
    ))
    .unwrap();
}

fn click(page: &mut PageVm, point: (f64, f64), count: i32, modifiers: u8) {
    mouse(page, "mousedown", point, count, modifiers);
    mouse(page, "mouseup", point, count, modifiers);
}

fn selection(page: &mut PageVm, control: &str) -> String {
    page.vm_mut()
        .eval(&format!(
            "[{control}.selectionStart,{control}.selectionEnd,{control}.selectionDirection].join(':')"
        ))
        .unwrap()
}

fn character_point(page: &mut PageVm, control: &str, index: usize) -> (f64, f64) {
    editor_point(page, control, EditorPoint::Character(index))
}

enum EditorPoint {
    Character(usize),
    AfterLine(usize),
}

fn editor_point(page: &mut PageVm, control: &str, point: EditorPoint) -> (f64, f64) {
    page.vm_mut()
        .with_default_context_scope_for_test(|_, host| {
            let host = unsafe { &*host };
            host.ensure_observable_layout()?;
            let control = host.dom_host().element_handle_by_id(control).unwrap();
            let point = host
                .with_latest_layout_tree_for_document(host.document_handle(), |tree| {
                    let control_box = tree.source_output(control).unwrap().principal_box.unwrap();
                    let fragment = match point {
                        EditorPoint::Character(index) => tree.fragments.iter().filter(|fragment| {
                            let moli_layout::LayoutFragmentKind::Text { box_id, .. } = fragment.kind else {
                                return false;
                            };
                            fragment.rect.width > 0.0
                                && tree.box_geometry(box_id)
                                    .and_then(|text| tree.box_geometry(text.parent?))
                                    .is_some_and(|editor| editor.parent == Some(control_box))
                        }).nth(index),
                        EditorPoint::AfterLine(index) => tree.fragments.iter().find(|fragment| {
                            matches!(fragment.kind, moli_layout::LayoutFragmentKind::Line {
                                owner, line_index, ..
                            } if line_index == index && tree.box_geometry(owner).is_some_and(|editor|
                                editor.parent == Some(control_box)))
                        }),
                    }.unwrap();
                    let x = match point {
                        EditorPoint::Character(_) => fragment.rect.x + fragment.rect.width * 0.2,
                        EditorPoint::AfterLine(_) => fragment.rect.right() + 10.0,
                    };
                    let local = moli_layout::LayoutPoint::new(x, fragment.rect.y + fragment.rect.height * 0.5);
                    tree.coordinate_space(fragment.coordinate_space)
                        .unwrap()
                        .local_to_viewport
                        .map_point(local)
                })
                .unwrap();
            Ok((f64::from(point.x), f64::from(point.y)))
        })
        .unwrap()
}

#[test]
fn text_control_pointer_places_caret_and_extends_a_selection() {
    run_page_vm_large_stack_async_test("text-control-pointer-selection", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = editor_page(&loader);
        let [two, three, six, nine, ten] =
            [2, 3, 6, 9, 10].map(|index| character_point(&mut page, "field", index));
        page.vm_mut().eval("field.focus(); field.select()").unwrap();
        mouse(&mut page, "mousedown", three, 1, 0);
        assert_eq!(selection(&mut page, "field"), "0:16:none");
        mouse(&mut page, "mouseup", three, 1, 0);
        assert_eq!(selection(&mut page, "field"), "3:3:none");

        mouse(&mut page, "mousedown", six, 1, 0);
        mouse(&mut page, "mousemove", ten, 0, 0);
        let (forward, backward, extended) = if cfg!(target_os = "macos") {
            ("none", "none", "0:9:none")
        } else {
            ("forward", "backward", "6:9:forward")
        };
        assert_eq!(selection(&mut page, "field"), format!("6:10:{forward}"));
        mouse(&mut page, "mousemove", (4.0, six.1), 0, 0);
        mouse(&mut page, "mouseup", (4.0, six.1), 1, 0);
        assert_eq!(selection(&mut page, "field"), format!("0:6:{backward}"));

        click(&mut page, nine, 1, 8);
        assert_eq!(selection(&mut page, "field"), extended);
        page.vm_mut()
            .eval(
                "field.addEventListener('mousedown', event => event.preventDefault(), {once:true})",
            )
            .unwrap();
        click(&mut page, two, 1, 0);
        assert_eq!(selection(&mut page, "field"), extended);

        page.vm_mut().eval("field.readOnly = true").unwrap();
        click(&mut page, two, 1, 0);
        assert_eq!(selection(&mut page, "field"), "2:2:none");
    });
}

#[test]
fn text_control_pointer_uses_editor_lines_and_password_value_offsets() {
    run_page_vm_large_stack_async_test("text-control-pointer-geometry", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = editor_page(&loader);
        let longer = character_point(&mut page, "area", 9);
        let bullet = character_point(&mut page, "secret", 2);
        let short_line = editor_point(&mut page, "area", EditorPoint::AfterLine(0));
        let blank_line = editor_point(&mut page, "area", EditorPoint::AfterLine(1));
        click(&mut page, short_line, 1, 0);
        assert_eq!(selection(&mut page, "area"), "5:5:none");
        click(&mut page, blank_line, 1, 0);
        assert_eq!(selection(&mut page, "area"), "6:6:none");
        click(&mut page, longer, 1, 0);
        assert_eq!(selection(&mut page, "area"), "11:11:none");

        click(&mut page, bullet, 1, 0);
        assert_eq!(selection(&mut page, "secret"), "3:3:none");
    });
}

#[test]
fn text_control_multiclick_selects_words_and_paragraphs() {
    run_page_vm_large_stack_async_test("text-control-multiclick", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = editor_page(&loader);
        let beta = character_point(&mut page, "field", 7);
        let gamma = character_point(&mut page, "field", 13);
        page.vm_mut()
            .eval("window.doubleClicks=[];field.addEventListener('dblclick', e => doubleClicks.push(e.detail))")
            .unwrap();
        click(&mut page, beta, 1, 0);
        click(&mut page, beta, 2, 0);
        assert_eq!(selected_value(&mut page, "field"), "beta");
        click(&mut page, beta, 3, 0);
        assert_eq!(selected_value(&mut page, "field"), "alpha beta gamma");
        assert_eq!(page.vm_mut().eval("doubleClicks.join(':')").unwrap(), "2");

        click(&mut page, beta, 1, 0);
        mouse(&mut page, "mousedown", beta, 2, 0);
        mouse(&mut page, "mousemove", gamma, 0, 0);
        mouse(&mut page, "mouseup", gamma, 2, 0);
        assert_eq!(selected_value(&mut page, "field"), "beta gamma");

        let short = character_point(&mut page, "area", 2);
        click(&mut page, short, 3, 0);
        assert_eq!(selected_value(&mut page, "area"), "short\n");
        let longer = character_point(&mut page, "area", 9);
        click(&mut page, longer, 3, 0);
        assert_eq!(selected_value(&mut page, "area"), "longer words");
    });
}

fn selected_value(page: &mut PageVm, control: &str) -> String {
    page.vm_mut()
        .eval(&format!(
            "{control}.value.slice({control}.selectionStart, {control}.selectionEnd)"
        ))
        .unwrap()
}

fn key(page: &mut PageVm, key: &str, modifiers: moli_input::InputModifiers) {
    page.vm_mut()
        .dispatch_key_event("keydown", key, key, "", modifiers.bits(), false, false)
        .unwrap();
}

#[test]
fn text_control_keyboard_moves_by_grapheme_word_and_line() {
    use moli_input::InputModifiers as Modifiers;
    run_page_vm_large_stack_async_test("text-control-keyboard-selection", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = editor_page(&loader);
        page.vm_mut()
            .eval("field.focus();field.setSelectionRange(0,0)")
            .unwrap();
        let word_modifier = if cfg!(target_os = "macos") {
            Modifiers::ALT
        } else {
            Modifiers::CONTROL
        };
        key(&mut page, "ArrowRight", word_modifier);
        let first_word_edge = if cfg!(target_os = "windows") {
            "6:6:none"
        } else {
            "5:5:none"
        };
        assert_eq!(selection(&mut page, "field"), first_word_edge);
        key(&mut page, "ArrowRight", word_modifier | Modifiers::SHIFT);
        let extended_word = if cfg!(target_os = "windows") {
            "beta "
        } else {
            " beta"
        };
        assert_eq!(selected_value(&mut page, "field"), extended_word);

        page.vm_mut()
            .eval("field.value='é😀Z';field.setSelectionRange(0,0)")
            .unwrap();
        key(&mut page, "ArrowRight", Modifiers::NONE);
        assert_eq!(selection(&mut page, "field"), "2:2:none");
        key(&mut page, "ArrowRight", Modifiers::SHIFT);
        assert_eq!(selected_value(&mut page, "field"), "😀");

        page.vm_mut()
            .eval("area.focus();area.setSelectionRange(11,11)")
            .unwrap();
        let (key_name, mods) = if cfg!(target_os = "macos") {
            ("ArrowLeft", Modifiers::META)
        } else {
            ("Home", Modifiers::NONE)
        };
        key(&mut page, key_name, mods);
        assert_eq!(selection(&mut page, "area"), "7:7:none");
        key(&mut page, "End", Modifiers::SHIFT);
        assert_eq!(selected_value(&mut page, "area"), "longer words");
    });
}
