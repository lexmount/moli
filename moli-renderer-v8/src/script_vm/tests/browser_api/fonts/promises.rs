use super::*;

#[test]
fn unobserved_font_face_errors_do_not_report_unhandled_rejections() {
    let mut vm = new_storage_test_vm("https://font-promises.test/");
    vm.eval(
        r#"
globalThis.rejections = [];
addEventListener('unhandledrejection', event => {
  rejections.push(event.reason.name);
  event.preventDefault();
});
globalThis.faces = [
  new FontFace('Invalid source', 'src'),
  new FontFace('Invalid descriptor', 'local("missing")', {weight: 'invalid'}),
  new FontFace('Invalid binary', new Uint8Array([0, 1, 0, 0]))
];
globalThis.event = new FontFaceSetLoadEvent('loadingerror', {fontfaces: faces});
"#,
    )
    .unwrap();
    vm.eval("0").unwrap();
    assert_eq!(
        vm.eval(
            "JSON.stringify([faces.map(face => face.status), event.fontfaces.length, rejections])"
        )
        .unwrap(),
        r#"[["error","error","error"],3,[]]"#
    );

    vm.eval("globalThis.errors = []; for (const face of faces) face.loaded.catch(error => errors.push(error.name))")
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([errors, rejections, faces.every(face => face.loaded === face.loaded && face.load() === face.loaded)])")
            .unwrap(),
        r#"[["SyntaxError","SyntaxError","SyntaxError"],[],true]"#
    );
}

#[test]
fn observed_font_face_errors_keep_promise_identity_and_rejection_notifications() {
    let mut vm = new_storage_test_vm("https://font-promises.test/");
    vm.eval(
        r#"
globalThis.events = [];
globalThis.faces = [new FontFace('Read', 'src'), new FontFace('Load', 'src')];
globalThis.promises = [faces[0].loaded, faces[1].load()];
addEventListener('unhandledrejection', event => {
  events.push([event.type, promises.indexOf(event.promise), event.reason.name]);
  event.preventDefault();
});
addEventListener('rejectionhandled', event => {
  events.push([event.type, promises.indexOf(event.promise), event.reason.name]);
});
"#,
    )
    .unwrap();
    vm.eval("0").unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[["unhandledrejection",0,"SyntaxError"],["unhandledrejection",1,"SyntaxError"]]"#
    );
    assert_eq!(
        vm.eval(
            "faces.every((face, i) => face.loaded === promises[i] && face.load() === promises[i])"
        )
        .unwrap(),
        "true"
    );
    vm.eval("promises.forEach(promise => promise.catch(() => {}))")
        .unwrap();
    vm.eval("0").unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(events.slice(2))").unwrap(),
        r#"[["rejectionhandled",0,"SyntaxError"],["rejectionhandled",1,"SyntaxError"]]"#
    );
}

#[test]
fn loaded_font_face_promise_can_be_observed_after_native_completion() {
    let mut vm = new_storage_test_vm("https://font-promises.test/");
    let data = fixture_font_url().split_once(',').unwrap().1.to_owned();
    vm.eval(&format!(
        r#"
globalThis.face = new FontFace('Binary', Uint8Array.from(atob('{data}'), c => c.charCodeAt(0)));
globalThis.result = [];
face.loaded.then(value => result.push(value === face, face.load() === face.loaded));
"#
    ))
    .unwrap();
    assert_eq!(vm.eval("JSON.stringify(result)").unwrap(), "[true,true]");
}
