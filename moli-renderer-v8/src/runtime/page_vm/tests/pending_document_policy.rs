use super::*;
#[cfg(feature = "devtools")]
use crate::runtime::RendererDevToolsPageCommand;

#[test]
fn pending_document_policy_rejects_another_agent_before_native_mutation() {
    let _runtime = crate::JsRuntime::initialize();
    let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
    let make_page = |id| {
        test_page_vm_with_id(
            PageId::new_for_testing(id),
            &loader,
            Vec::new(),
            DomHost::from_dom(HtmlParser::SCRIPTING_ENABLED.parse(
                Url::parse("https://example.com/").unwrap(),
                "<!doctype html><html><head></head><body></body></html>".to_owned(),
            )),
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
            None,
            None,
        )
    };
    let mut page = make_page(1);
    let other = make_page(2);
    let current_agent = page.devtools_agent_token();
    let other_agent = other.devtools_agent_token();
    assert_ne!(current_agent, other_agent);

    let current_document = page.document_lifecycle.identity();
    let other_document = other.document_lifecycle.identity();
    assert_ne!(current_document, other_document);
    for (expected_agent, source_document) in [
        (other_agent, other_document),
        (current_agent, current_document),
    ] {
        for update in [
            RendererPageCommand::SetBlockedUrlPatterns(vec!["*/pending".into()]),
            #[cfg(feature = "devtools")]
            RendererPageCommand::DevTools(
                RendererDevToolsPageCommand::SetFetchSubresourceInterception {
                    classes: crate::types::SubresourceInterceptionClasses::for_resource_type(
                        crate::types::SubresourceResourceType::Fetch,
                    ),
                },
            ),
        ] {
            let result = page.dispatch_renderer_page_command(RendererPageCommand::ForDocument {
                source_document,
                command: Box::new(update),
            });
            if expected_agent == other_agent {
                let Err(error) = result else {
                    panic!("a different agent must be rejected");
                };
                assert!(matches!(
                    error.downcast_ref::<crate::RendererPageAccessError>(),
                    Some(crate::RendererPageAccessError::DocumentReplaced)
                ));
                assert!(page.blocked_url_patterns.is_empty());
                assert!(!page.vm().fetch_subresource_interception_matches(
                    crate::types::SubresourceResourceType::Fetch
                ));
                assert!(!page.vm().fetch_subresource_interception_matches(
                    crate::types::SubresourceResourceType::Script
                ));
            } else {
                assert!(matches!(result.unwrap(), crate::RendererPageReply::Unit));
            }
        }
    }
    assert_eq!(page.blocked_url_patterns, ["*/pending"]);
    #[cfg(feature = "devtools")]
    assert!(
        page.vm()
            .fetch_subresource_interception_matches(crate::types::SubresourceResourceType::Fetch)
    );
    assert!(
        !page
            .vm()
            .fetch_subresource_interception_matches(crate::types::SubresourceResourceType::Script)
    );
}
