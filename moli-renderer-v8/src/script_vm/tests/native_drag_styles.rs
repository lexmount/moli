use super::*;
use crate::runtime::RendererPointerEventProperties;

#[test]
fn native_drag_eligibility_uses_the_cascade_including_draggable_hints() {
    for (source, css, expected_style, expected_drag) in [
        ("<div id=source>text</div>", "", "auto", false),
        (
            "<div id=source>text</div>",
            "#source{-webkit-user-drag:element}",
            "element",
            true,
        ),
        (
            "<div id=source draggable=true>text</div>",
            "#source{-webkit-user-drag:none}",
            "none",
            false,
        ),
        (
            "<div id=source draggable=false>text</div>",
            "#source{-webkit-user-drag:element}",
            "element",
            true,
        ),
        (
            "<div id=source draggable=true>text</div>",
            "#source{-webkit-user-drag:auto}",
            "auto",
            false,
        ),
        (
            "<a id=source href='/link'>link</a>",
            "#source{-webkit-user-drag:none}",
            "none",
            false,
        ),
        (
            "<a id=source href='/link' draggable=false>link</a>",
            "#source{-webkit-user-drag:auto}",
            "auto",
            true,
        ),
        (
            "<div id=source style='-webkit-user-drag:element'>text</div>",
            "#source{-webkit-user-drag:none!important}",
            "none",
            false,
        ),
        (
            "<div style='-webkit-user-drag:none'><a id=source href='/link'>link</a></div>",
            "",
            "auto",
            true,
        ),
        (
            "<div id=source>text</div>",
            "#source{--drag:element;-webkit-user-drag:var(--drag)}",
            "element",
            true,
        ),
        (
            "<div id=source draggable=true>text</div>",
            "#source{-webkit-user-drag:invalid}",
            "element",
            true,
        ),
    ] {
        let mut vm = new_rendered_test_vm(
            "https://drag-styles.test/",
            &format!(
                "<!doctype html><body style='margin:0'><style>#source{{display:block;width:100px;height:100px}}{css}</style>{source}"
            ),
        );
        vm.publish_layout_for_test().unwrap();
        assert_eq!(vm.eval("getComputedStyle(document.getElementById('source')).getPropertyValue('-webkit-user-drag')").unwrap(), expected_style, "{source}: {css}");
        vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, Some(1), 0.0, 0.0)
            .unwrap();
        let outcome = vm
            .dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                35.0,
                20.0,
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
            outcome.intercepted_drag.is_some(),
            expected_drag,
            "{source}: {css}"
        );
    }
}

#[test]
fn native_drag_observes_style_and_draggable_mutations_after_press() {
    for mutation in [
        "source.style.webkitUserDrag='none'",
        "source.draggable=false",
        "document.styleSheets[0].insertRule('#source{-webkit-user-drag:none}')",
    ] {
        let mut vm = new_rendered_test_vm(
            "https://drag-styles.test/",
            "<!doctype html><body style='margin:0'><style>#source{width:100px;height:100px}</style><div id=source draggable=true>source</div>",
        );
        vm.publish_layout_for_test().unwrap();
        vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, Some(1), 0.0, 0.0)
            .unwrap();
        vm.eval(mutation).unwrap();
        let outcome = vm
            .dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                35.0,
                20.0,
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
        assert!(outcome.intercepted_drag.is_none(), "{mutation}");
    }
}
