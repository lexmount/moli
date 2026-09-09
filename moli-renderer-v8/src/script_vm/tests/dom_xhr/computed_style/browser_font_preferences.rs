#![cfg(any(target_os = "linux", target_os = "freebsd"))]

use super::*;

#[test]
fn standard_font_preference_is_initial_style_not_an_author_overriding_rule() {
    let mut vm = new_parsed_test_vm(
        "https://browser-font-preferences.test/",
        r#"<!doctype html><body>
          <span id=default>Default</span>
          <div style="font-family: monospace">
            <span id=inherited>Inherited</span>
            <span id=initial style="font-family: initial">Initial</span>
          </div>
          <span id=serif style="font-family: serif">Serif</span>
          <span id=author style="font-family: Arial">Author</span>
        </body>"#,
    );
    let actual = vm
        .eval(
            r#"JSON.stringify(['default','inherited','initial','serif','author'].map(id =>
      getComputedStyle(document.getElementById(id)).fontFamily))"#,
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
        serde_json::json!([
            "\"Times New Roman\"",
            "monospace",
            "\"Times New Roman\"",
            "serif",
            "Arial"
        ])
    );
}
