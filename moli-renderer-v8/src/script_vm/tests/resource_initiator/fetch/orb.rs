use super::*;
use crate::types::SubresourceInterceptionClasses;

#[test]
fn fetch_resource_timing_orb_blocking_clears_tao_and_original_body_sizes() {
    let document = Url::parse("https://fetch-orb-timing.test/page").unwrap();
    let mut vm = new_storage_test_vm(document.as_str());
    vm.set_fetch_subresource_interception(SubresourceInterceptionClasses::for_resource_type(
        crate::types::SubresourceResourceType::Fetch,
    ));
    execute_with_source(
        &mut vm,
        "fetch('https://cross.test/blocked',{mode:'no-cors'}).then(r=>globalThis.response=r);",
        &document,
        &document,
    );
    let request = vm.take_pending_subresource_fetch_infos().pop().unwrap();
    finish_fetch(
        &mut vm,
        &request,
        crate::protocol_types::NavigationResponse::from_text_body(
            request.url.clone(),
            200,
            vec![
                ("Content-Type".into(), b"application/json".to_vec()),
                ("X-Content-Type-Options".into(), b"nosniff".to_vec()),
                ("Timing-Allow-Origin".into(), b"*".to_vec()),
            ],
            r#"{"blocked":"secret body"}"#.to_owned(),
        ),
        false,
    );
    assert_eq!(vm.eval("response.type").unwrap(), "opaque");
    assert_eq!(vm.eval("JSON.stringify(performance.getEntriesByType('resource').map(e=>[e.responseStatus,e.encodedBodySize,e.decodedBodySize,e.transferSize,e.contentType,e.initiatorUrl]))").unwrap(),r#"[[0,0,0,0,"",""]]"#);
}
