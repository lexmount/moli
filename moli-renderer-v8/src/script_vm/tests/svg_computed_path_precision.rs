use super::*;

#[test]
fn svg_computed_path_precision_survives_native_geometry_and_style_cascades() {
    let mut vm = new_parsed_test_vm(
        "https://svg-path-precision.test/",
        "<!doctype html><html><body><style id=geometry-style></style><svg xmlns='http://www.w3.org/2000/svg'><path d='M0 0L134217728 0'/></svg></body></html>",
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
      const path = document.querySelector('path');
      const check = (condition, label) => { if (!condition) throw Error(label); };
      check(path.getTotalLength() === 134217728, 'native computed attribute coordinates');
      const end = path.getPointAtLength(134217728);
      check(end.x === 134217728 && end.y === 0, 'native computed endpoint');
      const sheet = document.getElementById('geometry-style');
      sheet.textContent = 'path { d: path("M0 0L67108864 0"); }';
      check(path.getTotalLength() === 67108864, 'cascaded path overrides attribute');
      check(path.getAttribute('d') === 'M0 0L134217728 0', 'attribute unchanged');
      sheet.textContent = '';
      check(path.getTotalLength() === 134217728, 'attribute restored after mutation');
      return true;
    })()"#
        )
        .unwrap(),
        "true"
    );
}
