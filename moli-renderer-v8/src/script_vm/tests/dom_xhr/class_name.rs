use super::*;

#[test]
fn class_name_collections_use_the_receivers_current_document_mode() {
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/class-name-quirks.js"
    ));
    for (doctype, quirks) in [
        ("<!doctype html>", false),
        (
            "<!DOCTYPE HTML PUBLIC \"-//W3C//DTD XHTML 1.0 Transitional//EN\" \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd\">",
            false,
        ),
        ("", true),
    ] {
        let mut vm = new_parsed_test_vm(
            "https://class-name-quirks.test/",
            &format!("{doctype}<html><head></head><body></body></html>"),
        );
        let result = vm.eval(&format!(r#"{fixture}
(() => {{
  const result = classNameQuirksProbe({quirks});
  return JSON.stringify({{count: result.checks.length, failures: result.checks.filter(item => !item.pass)}});
}})()
"#)).expect("class collections should remain live across document modes and realms");
        assert_eq!(
            result,
            format!(
                r#"{{"count":{},"failures":[]}}"#,
                if quirks { 229 } else { 220 }
            ),
            "doctype={doctype}"
        );
    }
}
