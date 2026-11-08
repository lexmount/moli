use super::selection_order::{order_font_data, order_vm};

#[test]
fn font_face_descriptor_ranges_select_the_same_face_for_queries_and_layout() {
    let (narrow, wide) = order_font_data();
    for (property, range, other, request, changed) in [
        ("weight", "100 900", "400", "700", "100"),
        ("stretch", "75% 125%", "100%", "expanded", "75%"),
        (
            "style",
            "oblique 10deg 30deg",
            "oblique 20deg",
            "oblique 30deg",
            "oblique 10deg",
        ),
        (
            "weight",
            "calc(100 + 200) calc(800 + 100)",
            "700",
            "800",
            "100",
        ),
        (
            "stretch",
            "calc(50% + 25%) calc(100% + 25%)",
            "100%",
            "expanded",
            "75%",
        ),
        (
            "style",
            "oblique calc(5deg + 5deg) calc(15deg + 15deg)",
            "oblique 20deg",
            "oblique 30deg",
            "oblique 10deg",
        ),
        ("weight", "900 100", "400", "700", "100"),
        ("stretch", "125% 75%", "100%", "expanded", "75%"),
        (
            "style",
            "oblique 30deg 10deg",
            "oblique 20deg",
            "oblique 30deg",
            "oblique 10deg",
        ),
    ] {
        let mut vm = order_vm();
        vm.eval(&format!(
            r#"
            const bytes=data=>Uint8Array.from(atob(data),c=>c.charCodeAt(0));
            globalThis.good=new FontFace('OrderFace',bytes('{wide}'),{{{property}:'{range}'}});
            globalThis.other=new FontFace('OrderFace',bytes('{narrow}'),{{{property}:'{other}'}});
            document.fonts.add(good);document.fonts.add(other);
            probe.style.font='{request} 40px OrderFace';
            globalThis.selected=[];
            document.fonts.load('{request} 40px OrderFace','AAAA').then(
                faces=>selected.push(faces.length,faces[0]===good));
        "#
        ))
        .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify(selected)").unwrap(),
            "[1,true]",
            "{property}: {range}"
        );
        assert_eq!(
            vm.eval("String(probe.getBoundingClientRect().width)")
                .unwrap(),
            "160",
            "{property}: {range}"
        );
        vm.eval(&format!("good.{property}='{changed}'")).unwrap();
        assert_eq!(
            vm.eval("String(probe.getBoundingClientRect().width)")
                .unwrap(),
            "80",
            "{property}: descriptor mutation"
        );
    }
}

#[test]
fn font_face_overlapping_capability_ranges_follow_font_set_order() {
    let (narrow, wide) = order_font_data();
    let mut vm = order_vm();
    vm.eval(&format!(
        r#"
        const bytes=data=>Uint8Array.from(atob(data),c=>c.charCodeAt(0));
        globalThis.wide=new FontFace('OrderFace',bytes('{wide}'),{{weight:'100 900'}});
        globalThis.narrow=new FontFace('OrderFace',bytes('{narrow}'),{{weight:'500 800'}});
        document.fonts.add(wide);document.fonts.add(narrow);
        probe.style.font='700 40px OrderFace';
    "#
    ))
    .unwrap();
    assert_eq!(
        vm.eval("String(probe.getBoundingClientRect().width)")
            .unwrap(),
        "80"
    );
    vm.eval("document.fonts.delete(wide);document.fonts.add(wide)")
        .unwrap();
    assert_eq!(
        vm.eval("String(probe.getBoundingClientRect().width)")
            .unwrap(),
        "160"
    );
}
