use super::*;

#[test]
fn css_numeric_values_and_tokens_serialize_without_losing_double_precision() {
    let mut vm = new_parsed_test_vm(
        "https://css-numeric-serialization.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_numeric_serialization.js"))
            .unwrap(),
        "true"
    );
}

#[test]
fn css_unparsed_dimension_tokens_keep_their_type_and_value_when_reserialized() {
    let mut vm = new_storage_test_vm("https://css-token-boundaries.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
      for (const [source, expected] of [
        ['4e3e2', '4000\\65 2'], ['4e3\\65 2', '4000\\65 2'],
        ['1\\45 -2', '1\\45 -2'], ['1e21e2', '1e+21\\65 2'],
      ]) {
        const parsed = CSSStyleValue.parse('--token', source);
        if (parsed[0] !== expected || String(parsed) !== expected ||
            String(new CSSUnparsedValue([source])) !== expected ||
            String(CSSStyleValue.parse('--token', String(parsed))) !== expected)
          throw new Error('dimension changed: ' + source + ' -> ' + String(parsed));
      }
      return true;
    })()"#
        )
        .unwrap(),
        "true"
    );
}
