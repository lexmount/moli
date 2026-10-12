use super::*;
use moli_page_types::LayoutPolicy;

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
