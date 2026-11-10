use super::*;

#[test]
fn parser_resource_timing_uses_document_source_only_for_fresh_same_origin_responses() {
    use crate::types::SubresourceRequestInitiatorType::{Parser, Script};
    use moli_fetch::ResponseCacheState::{Local, None, Validated};
    let document = Url::parse("https://parser-timing.test/page").unwrap();
    for (initiator, cache, resource, expected) in [
        (
            Parser,
            None,
            "https://parser-timing.test/script.js",
            document.as_str(),
        ),
        (Parser, Local, "https://parser-timing.test/script.js", ""),
        (
            Parser,
            Validated,
            "https://parser-timing.test/script.js",
            "",
        ),
        (Parser, None, "https://other.test/script.js", ""),
        (Script, None, "https://parser-timing.test/script.js", ""),
    ] {
        let mut vm = new_storage_test_vm(document.as_str());
        let resource = Url::parse(resource).unwrap();
        let mut response = crate::protocol_types::NavigationResponse::from_text_body(
            resource.clone(),
            200,
            Vec::new(),
            "void 0".to_owned(),
        );
        response.cache_state = cache;
        vm.record_script_subresource_network_result_with_initiator(
            document.clone(),
            resource,
            initiator,
            &Ok(response),
        );
        assert_eq!(vm.eval("JSON.stringify(performance.getEntriesByType('resource').map(e => [e.initiatorUrl,e.toJSON().initiatorUrl]))").unwrap(),
            serde_json::json!([[expected,expected]]).to_string(), "{initiator:?}, {cache:?}");
    }
}
