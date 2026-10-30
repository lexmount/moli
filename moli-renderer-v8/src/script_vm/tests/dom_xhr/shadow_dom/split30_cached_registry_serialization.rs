use super::*;
#[test]
fn get_html_registry_serialization_tracks_template_owner_documents_and_option_mutations() {
    let mut vm = new_storage_test_vm("https://get-html-registry-documents.test/");
    let result = vm
        .eval(include_str!("../get_html_registry_documents.js"))
        .expect("serialize registries across template owner documents");
    let facts: serde_json::Value = serde_json::from_str(&result).expect("registry facts JSON");
    let rows = facts["rows"]
        .as_array()
        .expect("registry serialization cases");
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(row["passed"], true, "{row}");
    }
}
