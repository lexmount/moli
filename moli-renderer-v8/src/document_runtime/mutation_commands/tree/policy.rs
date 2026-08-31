#[derive(Clone, Copy)]
pub(super) enum TreeMutationSideEffectSource {
    JsDomApi,
    ParserTreeSink,
}

#[derive(Clone, Copy)]
pub(in crate::document_runtime::mutation_commands) enum TreeReactionDispatchPolicy {
    DispatchNow,
    AppendToCurrentQueue,
}

#[derive(Clone, Copy)]
pub(super) enum TreeNoncePolicy {
    HideInsertedContentAttributes,
    PreserveInsertedContentAttributes,
}

#[derive(Clone, Copy)]
pub(super) struct TreeMutationSourceProfile {
    pub(super) source: TreeMutationSideEffectSource,
    pub(super) reaction_policy: TreeReactionDispatchPolicy,
    pub(super) nonce_policy: TreeNoncePolicy,
    pub(super) sync_upgrade_connected_subtrees: bool,
    pub(super) queue_parser_details_toggle_events: bool,
    pub(super) queue_resource_followups: bool,
}

impl TreeNoncePolicy {
    pub(super) fn hides_inserted_content_attributes(self) -> bool {
        matches!(self, Self::HideInsertedContentAttributes)
    }
}

impl TreeMutationSourceProfile {
    pub(super) fn subresource_request_initiator_type(
        self,
    ) -> crate::types::SubresourceRequestInitiatorType {
        match self.source {
            TreeMutationSideEffectSource::ParserTreeSink => {
                crate::types::SubresourceRequestInitiatorType::Parser
            }
            TreeMutationSideEffectSource::JsDomApi => {
                crate::types::SubresourceRequestInitiatorType::Script
            }
        }
    }

    pub(super) fn js_dom_api_with(
        reaction_policy: TreeReactionDispatchPolicy,
        nonce_policy: TreeNoncePolicy,
    ) -> Self {
        Self {
            source: TreeMutationSideEffectSource::JsDomApi,
            reaction_policy,
            nonce_policy,
            sync_upgrade_connected_subtrees: true,
            queue_parser_details_toggle_events: false,
            queue_resource_followups: true,
        }
    }

    pub(super) fn js_dom_api() -> Self {
        Self::js_dom_api_with(
            TreeReactionDispatchPolicy::DispatchNow,
            TreeNoncePolicy::HideInsertedContentAttributes,
        )
    }

    pub(super) fn parser_tree_sink() -> Self {
        Self {
            source: TreeMutationSideEffectSource::ParserTreeSink,
            reaction_policy: TreeReactionDispatchPolicy::AppendToCurrentQueue,
            nonce_policy: TreeNoncePolicy::HideInsertedContentAttributes,
            sync_upgrade_connected_subtrees: false,
            queue_parser_details_toggle_events: true,
            queue_resource_followups: true,
        }
    }

    pub(super) fn child_parser_tree_sink() -> Self {
        Self {
            queue_resource_followups: false,
            ..Self::parser_tree_sink()
        }
    }

    pub(super) fn js_dom_api_appending_to_current_reaction_queue() -> Self {
        Self::js_dom_api_with(
            TreeReactionDispatchPolicy::AppendToCurrentQueue,
            TreeNoncePolicy::HideInsertedContentAttributes,
        )
    }

    pub(super) fn js_dom_api_preserving_nonce_appending_to_current_reaction_queue() -> Self {
        Self::js_dom_api_with(
            TreeReactionDispatchPolicy::AppendToCurrentQueue,
            TreeNoncePolicy::PreserveInsertedContentAttributes,
        )
    }

    pub(super) fn html_fragment_insertion_appending_to_current_reaction_queue() -> Self {
        Self {
            source: TreeMutationSideEffectSource::JsDomApi,
            reaction_policy: TreeReactionDispatchPolicy::AppendToCurrentQueue,
            nonce_policy: TreeNoncePolicy::HideInsertedContentAttributes,
            sync_upgrade_connected_subtrees: false,
            queue_parser_details_toggle_events: true,
            queue_resource_followups: true,
        }
    }
}
