use super::*;

const PARSER_FORM_HTML: &str = r#"<!doctype html><body>
<section id="cluster"><table id="carrier"><form id="owner">
<input id="radio" type="radio" name="group" checked></form></table></section>
<input id="peer" type="radio" name="group" checked><div id="destination"></div>"#;

#[test]
fn parser_form_owner_mutations_normalize_external_checked_radios() {
    for mutation in [
        "moveForm",
        "removeForm",
        "moveCarrier",
        "removeCarrier",
        "moveTogether",
        "removeTogether",
    ] {
        let mut vm = new_parsed_test_vm("https://form-owner-mutations.test/", PARSER_FORM_HTML);
        let script = format!(
            "({})({})",
            include_str!("owner_mutations_radios.js"),
            serde_json::to_string(mutation).unwrap()
        );
        assert_eq!(vm.eval(&script).unwrap(), "true", "{mutation}");
    }
}

#[test]
fn parser_form_owner_attribute_mutations_respect_namespace_and_case() {
    let mut vm = new_parsed_test_vm("https://form-owner-attributes.test/", PARSER_FORM_HTML);
    assert_eq!(
        vm.eval(include_str!("owner_mutations_attributes.js"))
            .unwrap(),
        "true"
    );
}

#[test]
fn secondary_document_form_collections_and_form_data_follow_owner_changes() {
    let mut vm = new_parsed_test_vm(
        "https://secondary-form-collections.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(
        vm.eval(include_str!("owner_mutations_secondary_document.js"))
            .unwrap(),
        "true"
    );
}
