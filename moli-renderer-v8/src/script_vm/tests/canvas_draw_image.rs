use super::*;

#[test]
fn native_draw_image_transforms_crop_and_sample_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-draw-image.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>';globalThis.drawImageRealms=[globalThis,document.querySelector('iframe').contentWindow];").unwrap();
    vm.eval(&format!(
        "const probe={};globalThis.drawImageChecks=[];for(let owner=0;owner<2;owner++)for(let callee=0;callee<2;callee++)for(const kind of ['element','offscreen'])drawImageChecks.push(...probe(drawImageRealms[owner],drawImageRealms[callee],kind,owner+'-'+callee+'-'+kind));",
        include_str!("canvas_draw_image.js")
    )).unwrap();
    assert_eq!(vm.eval("drawImageChecks.length").unwrap(), "400");
    assert_eq!(
        vm.eval("JSON.stringify(drawImageChecks.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}
