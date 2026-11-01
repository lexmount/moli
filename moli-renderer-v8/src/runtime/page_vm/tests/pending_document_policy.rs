use super::*;

#[test]
fn pending_document_policy_rejects_another_agent_before_native_mutation() {
    let _runtime = crate::JsRuntime::initialize();
    let mut page = test_page_vm_with_config(FetchConfig::default(), Vec::new());
    let other = test_page_vm_with_config(FetchConfig::default(), Vec::new());
    let current_agent = page.devtools_agent_token();
    let other_agent = other.devtools_agent_token();
    assert_ne!(current_agent, other_agent);

    for expected_agent in [other_agent, current_agent] {
        for update in [
            crate::RendererPageNetworkPolicyUpdate::BlockedUrlPatterns(vec!["*/pending".into()]),
            crate::RendererPageNetworkPolicyUpdate::FetchSubresourceInterception {
                enabled: true,
                resource_type: Some(crate::SubresourceResourceType::Fetch),
            },
        ] {
            let result = page.dispatch_renderer_page_command(
                RendererPageCommand::ForPendingDocumentPolicy {
                    expected_agent,
                    update,
                },
            );
            if expected_agent == other_agent {
                let Err(error) = result else {
                    panic!("a different agent must be rejected");
                };
                assert!(matches!(
                    error.downcast_ref::<crate::RendererPageAccessError>(),
                    Some(crate::RendererPageAccessError::DocumentReplaced)
                ));
                assert!(page.blocked_url_patterns.is_empty());
                assert!(
                    !page.vm().fetch_subresource_interception_matches(
                        crate::SubresourceResourceType::Fetch
                    )
                );
                assert!(!page.vm().fetch_subresource_interception_matches(
                    crate::SubresourceResourceType::Script
                ));
            } else {
                assert!(matches!(result.unwrap(), crate::RendererPageReply::Unit));
            }
        }
    }
    assert_eq!(page.blocked_url_patterns, ["*/pending"]);
    assert!(
        page.vm()
            .fetch_subresource_interception_matches(crate::SubresourceResourceType::Fetch)
    );
    assert!(
        !page
            .vm()
            .fetch_subresource_interception_matches(crate::SubresourceResourceType::Script)
    );
}
