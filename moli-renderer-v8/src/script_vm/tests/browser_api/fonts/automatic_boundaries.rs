use super::automatic_controls::font_usage_vm;
use super::selection_order::order_font_data;
use super::*;

#[test]
fn automatic_fonts_end_crlf_pairing_at_rendered_inline_objects() {
    let (_, wide) = order_font_data();
    for (object, expected) in [
        (
            "<i style='display:inline-block;width:5px;height:5px'></i>",
            "loaded",
        ),
        (
            "<i style='display:inline-flex;width:5px;height:5px'></i>",
            "loaded",
        ),
        (
            "<i style='display:inline-block;width:5px;height:5px'>x</i>",
            "loaded",
        ),
        ("<img width='5' height='5'>", "loaded"),
        ("<input value='x'>", "loaded"),
        ("<select><option>x</option></select>", "loaded"),
        ("<i style='float:left;width:5px;height:5px'></i>", "loaded"),
        // Out-of-flow objects and inline edges keep the first pending space's
        // font. Their LF still collapses with the CR's ordinary-font space.
        (
            "<i style='position:absolute;width:5px;height:5px'></i>",
            "unloaded",
        ),
        ("<i></i>", "unloaded"),
        ("<i style='display:contents'></i>", "unloaded"),
        ("<i style='display:none'></i>", "unloaded"),
    ] {
        let (mut vm, mut completions) = font_usage_vm();
        let object = serde_json::to_string(object).unwrap();
        vm.eval(&format!(r#"
            const container=document.createElement('div');
            container.append(document.createTextNode('a\r'));
            container.insertAdjacentHTML('beforeend',{object});
            const tail=document.createElement('span');
            tail.style.cssText='display:contents;font-family:SpaceFace';
            tail.textContent='\nb';container.append(tail);document.body.append(container);
            globalThis.face=new FontFace('SpaceFace','url(data:font/ttf;base64,{wide})',{{unicodeRange:'U+0020'}});
            document.fonts.add(face);
        "#)).unwrap();
        vm.reconcile_document_font_resources();
        vm.eval("document.fonts.ready.then(()=>globalThis.ready=true)")
            .unwrap();
        complete_local_font_resources(&mut vm, &mut completions);
        assert_eq!(vm.eval("face.status").unwrap(), expected, "{object}");
        assert_eq!(vm.eval("globalThis.ready===true").unwrap(), "true");
        assert_eq!(vm.layout_pass_observability_for_test().1, 0);
    }
}
